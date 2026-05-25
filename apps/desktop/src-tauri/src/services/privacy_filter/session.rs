//! OpenAI privacy-filter q4f16 ONNX 推理 session。
//!
//! 流程:
//! 1. **懒加载**:首次 [`PrivacySession::classify`] 调用时 load
//!    onnx + tokenizer + config.json,塞进 `RwLock<Option<Loaded>>`。
//!    后续 classify 走 read lock,零开销。
//! 2. **分词 + offset**:`tokenizers::Tokenizer::encode` 返回 token ids
//!    与每个 token 的 (byte_start, byte_end) 区间(原文坐标)。
//! 3. **推理**:`Session::run` 输入 `[input_ids, attention_mask]`
//!    (`int64 [1, seq]`),输出 `logits` (`float32 [1, seq, 33]`)。
//! 4. **解码**:`decode::decode_bioes` 把 logits 转成 token-level spans
//!    (greedy argmax + BIOES 状态机,见 decode.rs)。
//! 5. **char 映射**:把 token byte 区间转成 char 区间,输出
//!    [`PiiSpan`]。char 是因为 redact 需要 char-safe 中文场景。
//!
//! **错误策略**:任何 load / classify / decode 失败都退化成空 spans
//! 并 emit warning,不向上 panic —— privacy filter 是兜底层,失败时不
//! 应该挡住用户的 chat 出口。模型不在 / corrupt / 维度对不上,都是
//! "走原文出去"而不是"拒绝服务"。

use std::path::{Path, PathBuf};

use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::TensorRef;
use tokenizers::Tokenizer;
use tokio::sync::RwLock;

use crate::domain::privacy::PiiSpan;
use crate::error::{CorivoError, Result};

use super::decode::{decode_bioes, LabelMap};

/// 单 token argmax softmax 概率低于此值时,该 token 当 Outside 处理。
/// spec §4 推荐 0.3 起步;Phase 0 eval 决定是否调整。
const SCORE_THRESHOLD: f32 = 0.3;

/// 单次 forward 最长 token 数。模型 context window 是 128k,但
/// 实际 enforce 路径上的文本基本不超过几 KB —— 4096 token 是
/// 安全上限,超过的尾巴丢弃 + emit warning,后续支持滑窗。
const MAX_TOKENS: usize = 4096;

/// `PrivacySession` —— 模型生命周期容器。`PrivacyFilter` 持一份
/// `Arc<PrivacySession>`,在第一次 classify 时触发 load。
pub struct PrivacySession {
    model_dir: PathBuf,
    inner: RwLock<Option<Loaded>>,
}

struct Loaded {
    /// `Session::run` 需要 `&mut self`,但 `Loaded` 自己藏在外层
    /// `RwLock<Option<Loaded>>` 后面 —— 用 std::sync::Mutex 包一层让
    /// classify 路径可以拿可变引用。CPU-bound 推理本来就该串行
    /// (多线程并发跑 ONNX 也只会抢同一个 CPU 核),Mutex 是顺理成章。
    session: std::sync::Mutex<Session>,
    tokenizer: Tokenizer,
    label_map: LabelMap,
}

impl PrivacySession {
    /// 构造一个还未加载的 session。`model_dir` 指向
    /// `$APP_DATA/models/privacy-filter-q4f16/` (download.rs::model_dir
    /// 给出的路径)。
    pub fn new(model_dir: PathBuf) -> Self {
        Self {
            model_dir,
            inner: RwLock::new(None),
        }
    }

    /// 检查模型是否已经加载到内存。Settings UI 诊断面板用 —— 不影响
    /// classify 路径。
    pub async fn is_loaded(&self) -> bool {
        self.inner.read().await.is_some()
    }

    /// 卸载已加载的模型 —— "清除隐私缓存" / idle unload(后续)用。
    pub async fn unload(&self) {
        *self.inner.write().await = None;
    }

    /// 对 `text` 跑一次 PII classification,返回 char-level spans。
    ///
    /// 任何错误路径(model 文件缺失 / ort 抛 / 维度不符)都返回空 vec
    /// 并 emit warning。调用方应当处理"空 spans = 没有 PII 或模型不可
    /// 用"两种语义合一的情况。
    pub async fn classify(&self, text: &str) -> Vec<PiiSpan> {
        if text.is_empty() {
            return Vec::new();
        }

        // Lazy-load fast path: read lock, peek for Some.
        {
            let g = self.inner.read().await;
            if let Some(loaded) = g.as_ref() {
                return classify_with(loaded, text);
            }
        }

        // Slow path: upgrade to write lock and try to load.
        {
            let mut g = self.inner.write().await;
            if g.is_none() {
                match load_from_dir(&self.model_dir) {
                    Ok(loaded) => {
                        tracing::info!(
                            target: "privacy_filter",
                            model_dir = %self.model_dir.display(),
                            "session.loaded"
                        );
                        *g = Some(loaded);
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "privacy_filter",
                            ?error,
                            model_dir = %self.model_dir.display(),
                            "session.load_failed"
                        );
                        return Vec::new();
                    }
                }
            }
        }

        // Re-acquire read lock now that load succeeded.
        let g = self.inner.read().await;
        let loaded = match g.as_ref() {
            Some(l) => l,
            None => return Vec::new(),
        };
        classify_with(loaded, text)
    }
}

/// 从磁盘加载 onnx + tokenizer + config.json,组装成 `Loaded`。
///
/// 文件名跟 MODEL_MANIFEST 锁死 —— 用 `model_q4f16.*` 变体(770MB,
/// q4 block quantized + fp16 scale)。需要 ort >= 2.0.0-rc.12 ——
/// 早期的 rc.10 bundled ORT 不识别 `GatherBlockQuantized` op 的
/// `bits` attribute。Cargo.toml 里 `ort = "=2.0.0-rc.12"` 是锁死的,
/// 降级前务必更换模型变体(`model_quantized` INT8 1.5GB 是兜底)。
fn load_from_dir(model_dir: &Path) -> Result<Loaded> {
    let onnx_path = model_dir.join("model_q4f16.onnx");
    let tokenizer_path = model_dir.join("tokenizer.json");
    let config_path = model_dir.join("config.json");

    for p in [&onnx_path, &tokenizer_path, &config_path] {
        if !p.exists() {
            return Err(CorivoError::Internal(format!(
                "privacy filter file missing: {}",
                p.display()
            )));
        }
    }

    let session = Session::builder()
        .map_err(|e| CorivoError::Internal(format!("ort SessionBuilder: {e}")))?
        .with_optimization_level(GraphOptimizationLevel::Level1)
        .map_err(|e| CorivoError::Internal(format!("ort optimization_level: {e}")))?
        .commit_from_file(&onnx_path)
        .map_err(|e| {
            CorivoError::Internal(format!("ort commit_from_file {}: {e}", onnx_path.display()))
        })?;

    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| CorivoError::Internal(format!("tokenizer load: {e}")))?;

    let config_str = std::fs::read_to_string(&config_path)
        .map_err(|e| CorivoError::Internal(format!("config.json read: {e}")))?;
    let label_map = parse_label_map(&config_str)?;

    Ok(Loaded {
        session: std::sync::Mutex::new(session),
        tokenizer,
        label_map,
    })
}

/// 从 config.json 抽 `id2label` map,按 0..max_id 顺序排成数组,
/// 喂给 `LabelMap::from_id2label`。
fn parse_label_map(config_json: &str) -> Result<LabelMap> {
    let parsed: serde_json::Value = serde_json::from_str(config_json)
        .map_err(|e| CorivoError::Internal(format!("config.json parse: {e}")))?;
    let id2label = parsed
        .get("id2label")
        .and_then(|v| v.as_object())
        .ok_or_else(|| CorivoError::Internal("config.json missing id2label".to_string()))?;

    // id 是字符串 key,数值范围 [0, max]。
    let max_id = id2label
        .keys()
        .filter_map(|k| k.parse::<usize>().ok())
        .max()
        .ok_or_else(|| CorivoError::Internal("config.json id2label is empty".to_string()))?;

    let mut entries: Vec<String> = Vec::with_capacity(max_id + 1);
    for i in 0..=max_id {
        let raw = id2label
            .get(&i.to_string())
            .and_then(|v| v.as_str())
            .ok_or_else(|| CorivoError::Internal(format!("id2label missing entry {i}")))?;
        entries.push(raw.to_string());
    }
    let entries_ref: Vec<&str> = entries.iter().map(String::as_str).collect();
    Ok(LabelMap::from_id2label(&entries_ref))
}

/// 实际的 tokenize → run → decode → char-map 主流程。
fn classify_with(loaded: &Loaded, text: &str) -> Vec<PiiSpan> {
    match classify_inner(loaded, text) {
        Ok(spans) => spans,
        Err(error) => {
            tracing::warn!(
                target: "privacy_filter",
                ?error,
                text_len = text.len(),
                "session.classify_failed"
            );
            Vec::new()
        }
    }
}

fn classify_inner(loaded: &Loaded, text: &str) -> Result<Vec<PiiSpan>> {
    // 1. Tokenize. `add_special_tokens=true` —— 让 tokenizer 自己决定
    //    要不要加 CLS/SEP 这类特殊 token,offsets 给的是原文 byte 区间,
    //    特殊 token offsets 为 (0,0),decode 阶段映回 char 区间时会被
    //    "end<=start 跳过" 兜底过滤。
    let encoding = loaded
        .tokenizer
        .encode(text, true)
        .map_err(|e| CorivoError::Internal(format!("tokenize: {e}")))?;
    let ids = encoding.get_ids();
    let offsets = encoding.get_offsets();
    let mask = encoding.get_attention_mask();

    if ids.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Truncate to MAX_TOKENS. Long-form 文本 (>4k token) 走滑窗的
    //    工作放在后续 PR 里;目前直接 emit warning + 截尾。
    let n = ids.len().min(MAX_TOKENS);
    if ids.len() > MAX_TOKENS {
        tracing::warn!(
            target: "privacy_filter",
            total_tokens = ids.len(),
            kept_tokens = n,
            "session.input_truncated"
        );
    }

    // 3. Build input tensors. int64 [1, n].
    let input_ids: Vec<i64> = ids[..n].iter().map(|&x| x as i64).collect();
    let attention_mask: Vec<i64> = mask[..n].iter().map(|&x| x as i64).collect();
    let shape: [usize; 2] = [1, n];
    let input_ids_tensor = TensorRef::from_array_view((shape, input_ids.as_slice()))
        .map_err(|e| CorivoError::Internal(format!("ort input_ids tensor: {e}")))?;
    let attention_mask_tensor = TensorRef::from_array_view((shape, attention_mask.as_slice()))
        .map_err(|e| CorivoError::Internal(format!("ort attention_mask tensor: {e}")))?;

    // 4. Run inference + extract logits 一气呵成 —— `outputs` 借自
    //    session,`logits_view` 又借自 outputs,在 MutexGuard drop 之前
    //    必须把数据 clone 成 owned Vec。HF token-classification 模型
    //    固定输出名 `logits` —— OpenAI 的 ONNX 导出也走这个约定。
    let (shape, logits_data): (Vec<i64>, Vec<f32>) = {
        let mut session = loaded
            .session
            .lock()
            .map_err(|e| CorivoError::Internal(format!("session mutex poisoned: {e}")))?;
        let outputs = session
            .run(ort::inputs![
                "input_ids" => input_ids_tensor,
                "attention_mask" => attention_mask_tensor,
            ])
            .map_err(|e| CorivoError::Internal(format!("ort run: {e}")))?;
        let logits_value = outputs
            .get("logits")
            .ok_or_else(|| CorivoError::Internal("ort: missing `logits` output".to_string()))?;
        let (shape, view) = logits_value
            .try_extract_tensor::<f32>()
            .map_err(|e| CorivoError::Internal(format!("ort extract logits: {e}")))?;
        (shape.to_vec(), view.to_vec())
    };

    // 5. Validate shape & data length. Expect [1, seq, num_classes].
    if shape.len() != 3 {
        return Err(CorivoError::Internal(format!(
            "ort: unexpected logits shape {shape:?}, want [batch, seq, num_classes]"
        )));
    }
    let seq_len = shape[1] as usize;
    let num_classes = shape[2] as usize;
    if seq_len != n {
        return Err(CorivoError::Internal(format!(
            "ort: logits seq_len {seq_len} != input n {n}"
        )));
    }
    if logits_data.len() != n * num_classes {
        return Err(CorivoError::Internal(format!(
            "ort: logits data len {} != n*num_classes {}",
            logits_data.len(),
            n * num_classes
        )));
    }

    // 6. Reshape into Vec<Vec<f32>> for decode_bioes signature。
    //    Phase 0 eval 通过后可以改成 ndarray 减少拷贝,但 enforce 路径
    //    每次最多几 KB token,clone 是几十 KB 量级,先求清晰。
    let mut per_token: Vec<Vec<f32>> = Vec::with_capacity(n);
    for i in 0..n {
        let start = i * num_classes;
        let end = start + num_classes;
        per_token.push(logits_data[start..end].to_vec());
    }

    // 7. Decode BIOES → token-level spans.
    let token_spans = decode_bioes(&per_token, &loaded.label_map, SCORE_THRESHOLD);
    if token_spans.is_empty() {
        return Ok(Vec::new());
    }

    // 8. Map token byte offsets → char offsets。
    let byte_to_char = build_byte_to_char_map(text);
    let mut out = Vec::with_capacity(token_spans.len());
    for ts in token_spans {
        // 防御:decode_bioes 输出 token_start < token_end <= n,但这里
        // 仍然做 bounds check —— 异常 token 区间直接丢。
        if ts.token_end == 0 || ts.token_start >= n || ts.token_end > n {
            continue;
        }
        let (start_byte, _) = offsets[ts.token_start];
        let (_, end_byte) = offsets[ts.token_end - 1];
        if start_byte >= byte_to_char.len() || end_byte >= byte_to_char.len() {
            continue;
        }
        let start_char = byte_to_char[start_byte];
        let end_char = byte_to_char[end_byte];
        if end_char <= start_char {
            // 特殊 token (CLS/SEP/PAD) 的 offsets 是 (0,0),映射出来
            // 是空 range —— 跳过。其他模型输出异常 span 也走这里兜底。
            continue;
        }
        out.push(PiiSpan {
            start: start_char,
            end: end_char,
            label: ts.label,
            score: ts.score,
            redacted_in_storage: false,
        });
    }

    Ok(out)
}

/// 给原文构造 byte index → char index 查表。
///
/// `map[i] = text[..i].chars().count()`,长度 = `text.len() + 1`。
/// Tokenizer 给的 byte offsets 必然落在 UTF-8 char 边界上,但非边界
/// 位置(多字节字符中间)也给个保守值(前一个 char 的 index),避免
/// 越界 panic。
fn build_byte_to_char_map(text: &str) -> Vec<usize> {
    let mut map = vec![0usize; text.len() + 1];
    let mut char_idx = 0usize;
    let mut prev_char_start = 0usize;
    for (byte_idx, _) in text.char_indices() {
        // 上一个 char 起始字节 +1 到当前 char 起始字节之间的字节,都
        // 落在"上一个 char 的内部"。它们的 char index 是 char_idx - 1
        // (此时 char_idx 还没为当前 char 加 1,所以代表"已处理 char 数",
        // 减 1 就是上一个 char 的 index)。理论上 tokenizer offset
        // 不会指向 char 内部,这是防御兜底。
        if char_idx > 0 {
            for j in (prev_char_start + 1)..byte_idx {
                map[j] = char_idx - 1;
            }
        }
        map[byte_idx] = char_idx;
        prev_char_start = byte_idx;
        char_idx += 1;
    }
    // 末尾:最后一个 char 起始字节之后的所有字节,要么"在最后一个 char
    // 内部"(map 值 = 最后 char 的 index = char_idx - 1),要么"在 text
    // 末尾"(map[text.len()] = char_idx,即"所有 char 都跑完了之后的
    // 位置")。
    if char_idx > 0 {
        for j in (prev_char_start + 1)..text.len() {
            map[j] = char_idx - 1;
        }
    }
    map[text.len()] = char_idx;
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_to_char_pure_ascii() {
        let map = build_byte_to_char_map("abc");
        // len = 3 bytes + 1 = 4
        assert_eq!(map.len(), 4);
        assert_eq!(map, vec![0, 1, 2, 3]);
    }

    #[test]
    fn byte_to_char_with_cjk() {
        // "ab中" = a(1B) + b(1B) + 中(3B) = 5 bytes, 3 chars
        let map = build_byte_to_char_map("ab中");
        assert_eq!(map.len(), 6);
        // 0='a' start (char 0), 1='b' start (char 1), 2='中' start (char 2),
        // 3,4 = inside 中 (保守填 2), 5 = end (char 3)
        assert_eq!(map[0], 0);
        assert_eq!(map[1], 1);
        assert_eq!(map[2], 2);
        assert_eq!(map[3], 2);
        assert_eq!(map[4], 2);
        assert_eq!(map[5], 3);
    }

    #[test]
    fn byte_to_char_empty_string() {
        let map = build_byte_to_char_map("");
        assert_eq!(map, vec![0]);
    }

    #[test]
    fn parse_label_map_real_config() {
        // 真实 config.json 的精简版 —— 至少覆盖 id 0..4 的解析。
        let json = r#"{
            "id2label": {
                "0": "O",
                "1": "B-account_number",
                "2": "I-account_number",
                "3": "E-account_number",
                "4": "S-account_number"
            }
        }"#;
        let map = parse_label_map(json).unwrap();
        assert_eq!(map.class_count(), 5);
    }

    #[test]
    fn parse_label_map_missing_field_errors() {
        let json = r#"{ "something_else": {} }"#;
        let err = parse_label_map(json).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("id2label"), "expected mention of id2label, got: {msg}");
    }

    #[tokio::test]
    async fn classify_with_missing_model_dir_returns_empty() {
        let session = PrivacySession::new(PathBuf::from("/nonexistent/path"));
        let spans = session.classify("hello world").await;
        assert!(spans.is_empty());
        assert!(!session.is_loaded().await);
    }

    #[tokio::test]
    async fn classify_empty_text_returns_empty_without_load() {
        let session = PrivacySession::new(PathBuf::from("/also/nonexistent"));
        let spans = session.classify("").await;
        assert!(spans.is_empty());
        // 空文本短路不应该触发 load 尝试。
        assert!(!session.is_loaded().await);
    }

    /// 真实模型烟测试 —— 默认 ignored,因为依赖外部 800MB 模型文件。
    ///
    /// 跑法:
    /// 1. 把 q4f16 模型先下下来(`pnpm app:dev` 进 Settings 触发,
    ///    或者用 download.rs::ensure_downloaded 写个手动小脚本)。
    /// 2. 设环境变量 `CORIVO_PRIVACY_MODEL_DIR` 指向那个目录,
    ///    比如 `$env:CORIVO_PRIVACY_MODEL_DIR = "$env:TEMP\corivo-privacy-filter"`。
    /// 3. `cargo test --lib --include-ignored real_model_classifies`。
    ///
    /// 期望:对一段含明显 PII 的文本(邮箱 + 人名)返回至少一个 span,
    /// label 命中 PrivateEmail 或 PrivatePerson 之一。
    #[tokio::test]
    #[ignore = "depends on external 800MB model files; set CORIVO_PRIVACY_MODEL_DIR to enable"]
    async fn real_model_classifies_email_pii() {
        let dir = match std::env::var("CORIVO_PRIVACY_MODEL_DIR") {
            Ok(d) => PathBuf::from(d),
            Err(_) => {
                eprintln!("CORIVO_PRIVACY_MODEL_DIR not set; skipping real-model test");
                return;
            }
        };
        if !dir.join("model_q4f16.onnx").exists() {
            eprintln!(
                "model files not at {} ; skipping real-model test",
                dir.display()
            );
            return;
        }
        let session = PrivacySession::new(dir);
        let text = "Please email me at alice@example.com about the project.";
        let spans = session.classify(text).await;
        eprintln!("real-model classify on {text:?}");
        eprintln!("  produced {} spans:", spans.len());
        for s in &spans {
            eprintln!(
                "    [{:>2}..{:<2}] {:?} (score={:.3})",
                s.start, s.end, s.label, s.score
            );
        }
        assert!(
            !spans.is_empty(),
            "expected at least one PII span from real model on text with email, got none"
        );
    }

    /// 诊断:跑一次真实模型,直接 dump 前 5 个 token 的 argmax + 分数。
    /// 用来定位"是 inference 没产 logit 还是 decode 阈值过严"。
    #[tokio::test]
    #[ignore = "diagnostic only; requires CORIVO_PRIVACY_MODEL_DIR"]
    async fn diagnose_raw_logits() {
        let dir = match std::env::var("CORIVO_PRIVACY_MODEL_DIR") {
            Ok(d) => PathBuf::from(d),
            Err(_) => return,
        };
        if !dir.join("model_q4f16.onnx").exists() {
            return;
        }

        // 复刻 classify 但拦截 logits 出来打印。
        let loaded = load_from_dir(&dir).expect("model load");
        let text = "Please email me at alice@example.com about the project.";
        let encoding = loaded.tokenizer.encode(text, true).expect("tokenize");
        let ids = encoding.get_ids();
        let offsets = encoding.get_offsets();
        let mask = encoding.get_attention_mask();
        eprintln!("tokenizer produced {} tokens:", ids.len());
        for (i, (id, off)) in ids.iter().zip(offsets.iter()).enumerate().take(20) {
            let tok_text = if off.0 == 0 && off.1 == 0 {
                "<special>".to_string()
            } else {
                text[off.0..off.1].to_string()
            };
            eprintln!("  [{i:>3}] id={id:>6} offset=({:>2},{:>2}) text={tok_text:?}", off.0, off.1);
        }
        let n = ids.len();

        let input_ids: Vec<i64> = ids.iter().map(|&x| x as i64).collect();
        let attention_mask: Vec<i64> = mask.iter().map(|&x| x as i64).collect();
        let shape: [usize; 2] = [1, n];
        let in_ids = TensorRef::from_array_view((shape, input_ids.as_slice())).unwrap();
        let in_mask = TensorRef::from_array_view((shape, attention_mask.as_slice())).unwrap();

        let (sh, data): (Vec<i64>, Vec<f32>) = {
            let mut session = loaded.session.lock().unwrap();
            let outputs = session
                .run(ort::inputs![
                    "input_ids" => in_ids,
                    "attention_mask" => in_mask,
                ])
                .expect("ort run");
            let logits = outputs
                .get("logits")
                .expect("missing logits output");
            let (sh, v) = logits.try_extract_tensor::<f32>().expect("extract");
            (sh.to_vec(), v.to_vec())
        };
        eprintln!("output shape: {sh:?}");
        let num_classes = sh[2] as usize;
        eprintln!("label_map has {} classes", loaded.label_map.class_count());

        // 打印前 20 个 token 的 top-3 argmax + softmax
        for i in 0..n.min(20) {
            let start = i * num_classes;
            let end = start + num_classes;
            let row = &data[start..end];
            let max_v = row.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            let mut sum = 0.0f32;
            for &v in row {
                if v.is_finite() {
                    sum += (v - max_v).exp();
                }
            }
            let mut scored: Vec<(usize, f32)> = row
                .iter()
                .enumerate()
                .map(|(c, &v)| (c, ((v - max_v).exp()) / sum))
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            let top3: Vec<String> = scored
                .iter()
                .take(3)
                .map(|(c, p)| format!("{c}={p:.3}"))
                .collect();
            let off = offsets[i];
            let tok_text = if off.0 == 0 && off.1 == 0 {
                "<sp>".to_string()
            } else {
                text[off.0..off.1].to_string()
            };
            eprintln!("  tok[{i:>2}] {tok_text:>20?}  top3: {}", top3.join(", "));
        }
    }
}
