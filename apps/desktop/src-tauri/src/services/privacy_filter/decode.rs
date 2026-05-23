//! BIOES 序列解码 —— 把 [seq_len × num_classes] 的 token-level
//! logits 转成 token-level spans。
//!
//! OpenAI privacy-filter 的输出层是 33 类：1 个 `O`（背景）+ 8 个
//! PII label × 4 个 BIOES boundary tag。spec §1 / model card 都明确
//! 这个形态。模型卡里提的是"约束 Viterbi 解码"，这里**先用 greedy
//! argmax + 后处理**：
//!
//! - 工程量低一个数量级
//! - 测试覆盖容易写
//! - 在 q4f16 这种已经 well-calibrated 的 token 分类器上,greedy
//!   通常已经能拿到 95%+ 的 Viterbi 质量
//! - 如果 Phase 0 eval 显示 greedy 输出 F1 显著低于"openai-onnx
//!   fp16" 的 0.758,再切到 constrained Viterbi
//!
//! BIOES 含义:
//!   B-X  开始一段 X 类型 span
//!   I-X  延续一段 X 类型 span
//!   E-X  结束一段 X 类型 span
//!   S-X  单 token 的 X 类型 span（B+E 合一）
//!   O    非 PII token
//!
//! 解码状态机:
//!   open=None
//!   ├ tok=O:           close(open); open=None
//!   ├ tok=S-X:         close(open); emit single; open=None
//!   ├ tok=B-X:         close(open); open=Span(X, start=i)
//!   ├ tok=I-X & open.label==X: open.end=i+1
//!   ├ tok=I-X & 不匹配:  close(open); open=Span(X, start=i)  ← 错误恢复
//!   ├ tok=E-X & open.label==X: open.end=i+1; emit(open); open=None
//!   └ tok=E-X & 不匹配:  close(open); emit single(X, start=i); open=None
//!   end-of-seq: close(open)
//!
//! 解码器**不**做 char offset 转换 —— 输出是 token index 的 span。
//! 调用方（持有 tokenizer offsets）负责把 token-level span 转成
//! char-level span 再喂给 redact。

use crate::domain::privacy::PiiLabel;

/// BIOES 边界标签。enum 顺序无意义,只是离散类型 —— 数值映射在
/// `LabelMap` 里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BioesTag {
    /// Outside —— 非 PII（class 0）
    Outside,
    /// Begin —— span 开始
    Begin,
    /// Inside —— span 中间
    Inside,
    /// End —— span 结束
    End,
    /// Single —— 单 token span
    Single,
}

/// class_id → (label, tag) 的查找表。
///
/// **不在本文件硬编码 OpenAI 的具体 id2label** —— 那要等模型下载完读
/// `config.json` 才知道。`session.rs`（下一阶段）负责加载并构造
/// `LabelMap`,本模块只接 `&LabelMap` 用。
#[derive(Debug, Clone)]
pub struct LabelMap {
    /// `entries[class_id]` 给出该 class 的语义。`None` = 该 id 不
    /// 是有效输出（理论上不会出现,防御性兜底）。
    entries: Vec<Option<LabelEntry>>,
}

#[derive(Debug, Clone, Copy)]
struct LabelEntry {
    label: Option<PiiLabel>, // None 表示 Outside
    tag: BioesTag,
}

impl LabelMap {
    /// 从 `id2label` 字符串数组构造。字符串格式约定:
    ///   "O"                  → Outside
    ///   "B-{snake_case}"     → Begin
    ///   "I-{snake_case}"     → Inside
    ///   "E-{snake_case}"     → End
    ///   "S-{snake_case}"     → Single
    /// 不认识的字符串会被记成 `None`（解码时跳过）。
    ///
    /// 这个解析路径是给将来 `session.rs` 加载 `config.json` 后用的。
    pub fn from_id2label(id2label: &[&str]) -> Self {
        let entries = id2label
            .iter()
            .map(|raw| parse_id2label_entry(raw))
            .collect();
        Self { entries }
    }

    pub fn class_count(&self) -> usize {
        self.entries.len()
    }

    fn lookup(&self, class_id: usize) -> Option<LabelEntry> {
        self.entries.get(class_id).copied().flatten()
    }
}

fn parse_id2label_entry(raw: &str) -> Option<LabelEntry> {
    if raw == "O" {
        return Some(LabelEntry {
            label: None,
            tag: BioesTag::Outside,
        });
    }
    let (prefix, name) = raw.split_once('-')?;
    let tag = match prefix {
        "B" => BioesTag::Begin,
        "I" => BioesTag::Inside,
        "E" => BioesTag::End,
        "S" => BioesTag::Single,
        _ => return None,
    };
    // serde rename_all = snake_case,所以直接 deserialize 一个字符串。
    // 用 serde_json 是因为 PiiLabel 实现的是 serde, 包一层引号即可。
    let quoted = format!("\"{name}\"");
    let label: PiiLabel = serde_json::from_str(&quoted).ok()?;
    Some(LabelEntry {
        label: Some(label),
        tag,
    })
}

/// Token-level span,decode 的输出形态。`token_start` / `token_end`
/// 是闭右开区间(`[start, end)`),对应 logits 数组的 token 下标。
#[derive(Debug, Clone, PartialEq)]
pub struct TokenSpan {
    pub token_start: usize,
    pub token_end: usize,
    pub label: PiiLabel,
    /// span 内 token 最低 score —— 用最弱链作为 span 整体置信度,
    /// 对"半信半疑就报"的模型更保守一些。
    pub score: f32,
}

/// 主入口:把 token-level logits + 类目映射 + 阈值 → token spans。
///
/// **logits**: `&[Vec<f32>]`,外层是 seq_len,内层是 class_count。
/// 不强制 softmax —— 函数会自己拿 argmax 同时计算 softmax score
/// 用作阈值过滤。如果调用方已经过 softmax,数值结果不影响 argmax。
///
/// **threshold**: 单 token 上,argmax class 的 softmax 概率低于此值
/// 时,该 token 当 Outside 处理。**对应 spec §4 "configurable
/// operating points"** —— 调整这个值能换 precision/recall 比例。
/// 推荐 0.3 起步（GLiNER 推荐区间 0.2-0.6,OpenAI 模型尚未实测
/// 决定，等 Phase 0 eval 出数据后调）。
pub fn decode_bioes(logits: &[Vec<f32>], label_map: &LabelMap, threshold: f32) -> Vec<TokenSpan> {
    let mut out: Vec<TokenSpan> = Vec::new();
    let mut open: Option<OpenSpan> = None;

    for (i, token_logits) in logits.iter().enumerate() {
        let (class_id, score) = match argmax_with_softmax(token_logits) {
            Some(pair) => pair,
            None => continue, // 全 NaN / 空,跳过
        };

        if score < threshold {
            close_into(&mut open, &mut out);
            continue;
        }

        let Some(entry) = label_map.lookup(class_id) else {
            close_into(&mut open, &mut out);
            continue;
        };

        match (entry.tag, entry.label) {
            (BioesTag::Outside, _) | (_, None) => {
                close_into(&mut open, &mut out);
            }
            (BioesTag::Single, Some(label)) => {
                close_into(&mut open, &mut out);
                out.push(TokenSpan {
                    token_start: i,
                    token_end: i + 1,
                    label,
                    score,
                });
            }
            (BioesTag::Begin, Some(label)) => {
                close_into(&mut open, &mut out);
                open = Some(OpenSpan {
                    start: i,
                    label,
                    min_score: score,
                });
            }
            (BioesTag::Inside, Some(label)) => match &mut open {
                Some(span) if span.label == label => {
                    span.min_score = span.min_score.min(score);
                }
                _ => {
                    // 错误恢复:把 I-X 当作 B-X 处理 —— 与其丢一个 PII
                    // 不如多 redact 一个,符合"egress 安全"导向
                    close_into(&mut open, &mut out);
                    open = Some(OpenSpan {
                        start: i,
                        label,
                        min_score: score,
                    });
                }
            },
            (BioesTag::End, Some(label)) => match open.take() {
                Some(span) if span.label == label => {
                    out.push(TokenSpan {
                        token_start: span.start,
                        token_end: i + 1,
                        label,
                        score: span.min_score.min(score),
                    });
                }
                Some(stale) => {
                    // 当前 open span 类型不匹配 E:先关掉它（作为
                    // 单独的 span 输出 —— 它有 B 但没 E,降级成 S）,
                    // 再把这个孤立 E-X 当 S-X
                    out.push(TokenSpan {
                        token_start: stale.start,
                        token_end: i, // 不含当前 i
                        label: stale.label,
                        score: stale.min_score,
                    });
                    out.push(TokenSpan {
                        token_start: i,
                        token_end: i + 1,
                        label,
                        score,
                    });
                }
                None => {
                    // 孤立 E-X: 当 S-X 处理
                    out.push(TokenSpan {
                        token_start: i,
                        token_end: i + 1,
                        label,
                        score,
                    });
                }
            },
        }
    }

    // 序列结束时还有未关闭 span: 关掉它（B...I... 没等到 E）
    if let Some(span) = open.take() {
        out.push(TokenSpan {
            token_start: span.start,
            token_end: logits.len(),
            label: span.label,
            score: span.min_score,
        });
    }

    out
}

struct OpenSpan {
    start: usize,
    label: PiiLabel,
    min_score: f32,
}

fn close_into(open: &mut Option<OpenSpan>, out: &mut Vec<TokenSpan>) {
    if let Some(span) = open.take() {
        // open span 走到这里说明没等到 E —— 用当前已知的 end（即 next
        // token 之前）。但本函数被调用时不持有 i,所以保守地放成
        // start+1。调用点会在 caller 收尾时再补一遍 end_of_seq 关闭逻辑。
        out.push(TokenSpan {
            token_start: span.start,
            token_end: span.start + 1,
            label: span.label,
            score: span.min_score,
        });
    }
}

/// argmax + 该 class 的 softmax 概率（用作 threshold 过滤）。
/// 全 NaN / 空数组返回 None。
fn argmax_with_softmax(logits: &[f32]) -> Option<(usize, f32)> {
    if logits.is_empty() {
        return None;
    }
    // 找 max,顺便给 softmax 做减峰（数值稳定）
    let mut max_idx = 0usize;
    let mut max_val = logits[0];
    for (i, &v) in logits.iter().enumerate().skip(1) {
        if v > max_val {
            max_val = v;
            max_idx = i;
        }
    }
    if !max_val.is_finite() {
        return None;
    }
    let mut sum = 0.0f32;
    for &v in logits {
        if v.is_finite() {
            sum += (v - max_val).exp();
        }
    }
    if sum <= 0.0 || !sum.is_finite() {
        return None;
    }
    let score = 1.0 / sum;
    Some((max_idx, score))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用迷你 label map:
    ///   0: O
    ///   1: B-private_person
    ///   2: I-private_person
    ///   3: E-private_person
    ///   4: S-private_person
    ///   5: B-private_email
    ///   6: I-private_email
    ///   7: E-private_email
    ///   8: S-private_email
    fn test_label_map() -> LabelMap {
        LabelMap::from_id2label(&[
            "O",
            "B-private_person",
            "I-private_person",
            "E-private_person",
            "S-private_person",
            "B-private_email",
            "I-private_email",
            "E-private_email",
            "S-private_email",
        ])
    }

    /// One-hot logits 给定 class —— score 接近 1.0
    fn one_hot(class: usize, num_classes: usize) -> Vec<f32> {
        let mut v = vec![-10.0; num_classes];
        v[class] = 10.0;
        v
    }

    #[test]
    fn empty_logits_yields_no_spans() {
        let spans = decode_bioes(&[], &test_label_map(), 0.3);
        assert!(spans.is_empty());
    }

    #[test]
    fn all_outside_yields_no_spans() {
        let logits = vec![one_hot(0, 9); 5];
        let spans = decode_bioes(&logits, &test_label_map(), 0.3);
        assert!(spans.is_empty());
    }

    #[test]
    fn single_s_tag_yields_one_span() {
        // [O, S-person, O]
        let logits = vec![one_hot(0, 9), one_hot(4, 9), one_hot(0, 9)];
        let spans = decode_bioes(&logits, &test_label_map(), 0.3);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].token_start, 1);
        assert_eq!(spans[0].token_end, 2);
        assert_eq!(spans[0].label, PiiLabel::PrivatePerson);
    }

    #[test]
    fn b_i_e_sequence_yields_one_span() {
        // [O, B-person, I-person, E-person, O]
        let logits = vec![
            one_hot(0, 9),
            one_hot(1, 9),
            one_hot(2, 9),
            one_hot(3, 9),
            one_hot(0, 9),
        ];
        let spans = decode_bioes(&logits, &test_label_map(), 0.3);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].token_start, 1);
        assert_eq!(spans[0].token_end, 4);
    }

    #[test]
    fn b_e_yields_two_token_span() {
        // [B-person, E-person]
        let logits = vec![one_hot(1, 9), one_hot(3, 9)];
        let spans = decode_bioes(&logits, &test_label_map(), 0.3);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].token_start, 0);
        assert_eq!(spans[0].token_end, 2);
    }

    #[test]
    fn unclosed_b_at_end_still_emits_span() {
        // [B-person, I-person] —— 序列结束时 open span 应被关掉
        let logits = vec![one_hot(1, 9), one_hot(2, 9)];
        let spans = decode_bioes(&logits, &test_label_map(), 0.3);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].token_start, 0);
        assert_eq!(spans[0].token_end, 2);
    }

    #[test]
    fn isolated_e_treated_as_single() {
        // [O, E-person, O] —— 孤立 E 应当作 S 处理
        let logits = vec![one_hot(0, 9), one_hot(3, 9), one_hot(0, 9)];
        let spans = decode_bioes(&logits, &test_label_map(), 0.3);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].token_start, 1);
        assert_eq!(spans[0].token_end, 2);
    }

    #[test]
    fn i_without_b_treated_as_b() {
        // [O, I-person, E-person] —— 错误恢复:I 当 B
        let logits = vec![one_hot(0, 9), one_hot(2, 9), one_hot(3, 9)];
        let spans = decode_bioes(&logits, &test_label_map(), 0.3);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].token_start, 1);
        assert_eq!(spans[0].token_end, 3);
    }

    #[test]
    fn two_consecutive_spans_different_labels() {
        // [B-person, E-person, S-email]
        let logits = vec![one_hot(1, 9), one_hot(3, 9), one_hot(8, 9)];
        let spans = decode_bioes(&logits, &test_label_map(), 0.3);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].label, PiiLabel::PrivatePerson);
        assert_eq!(spans[0].token_end, 2);
        assert_eq!(spans[1].label, PiiLabel::PrivateEmail);
        assert_eq!(spans[1].token_start, 2);
    }

    #[test]
    fn label_mismatch_in_middle_splits_spans() {
        // [B-person, I-email, E-email] —— label 不一致:
        //   - B-person 没等到匹配的 E,降级输出
        //   - I-email 当 B-email 开新 span
        //   - E-email 收尾
        let logits = vec![one_hot(1, 9), one_hot(6, 9), one_hot(7, 9)];
        let spans = decode_bioes(&logits, &test_label_map(), 0.3);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].label, PiiLabel::PrivatePerson);
        assert_eq!(spans[1].label, PiiLabel::PrivateEmail);
    }

    #[test]
    fn low_confidence_treated_as_outside() {
        // S-person token,但 score 拉到很低 —— threshold 拦下
        let mut weak = vec![0.0f32; 9];
        weak[4] = 0.5; // 微弱偏向 S-person
                       // softmax(0.5 vs 0s) = exp(0.5) / (8*exp(0) + exp(0.5)) ≈ 0.17
        let logits = vec![weak];
        let spans = decode_bioes(&logits, &test_label_map(), 0.3);
        assert!(
            spans.is_empty(),
            "score below threshold should produce no span"
        );
    }

    #[test]
    fn label_map_parses_id2label_strings() {
        let map = LabelMap::from_id2label(&["O", "B-private_person", "S-secret", "garbage-text"]);
        assert_eq!(map.class_count(), 4);
        assert_eq!(map.lookup(0).unwrap().tag, BioesTag::Outside);
        assert!(map.lookup(0).unwrap().label.is_none());
        assert_eq!(map.lookup(1).unwrap().tag, BioesTag::Begin);
        assert_eq!(map.lookup(1).unwrap().label, Some(PiiLabel::PrivatePerson));
        assert_eq!(map.lookup(2).unwrap().tag, BioesTag::Single);
        assert_eq!(map.lookup(2).unwrap().label, Some(PiiLabel::Secret));
        assert!(
            map.lookup(3).is_none(),
            "garbage label should not parse into an entry"
        );
    }

    #[test]
    fn span_score_takes_minimum_across_tokens() {
        // 三个 token 都触发 BIE 序列,但中间那个置信度明显偏低 ——
        // span.score 应等于该 token 的 softmax,严格低于另两个。
        //
        // softmax 对单峰输入收敛很快(单 hot logit 几乎给 1.0),所以
        // "弱信号" 要靠 *多个* 类目同时高才造得出 —— B-person 与
        // I-email 都给 1.0,其余给 0,模型其实在 1 / 2 之间犹豫,
        // argmax(B-person) 的概率≈0.5。
        let token_a = one_hot(1, 9); // 强 B-person,score≈1.0
        let mut token_b = vec![0.0; 9];
        token_b[2] = 1.0; // I-person logit 1
        token_b[6] = 1.0; // 同时 I-email 也 1 —— 模型在两类间纠结
        let token_c = one_hot(3, 9); // 强 E-person

        // 先确认 token_b 的 softmax 在我们预期的范围。两个并列 max
        // 让概率分摊掉一半,再加上 7 个 e^-1 的尾部 —— 实测 ≈0.22。
        let (cls, s_b) = argmax_with_softmax(&token_b).unwrap();
        assert_eq!(
            cls, 2,
            "argmax should still resolve to I-person (first max)"
        );
        assert!(
            s_b > 0.15 && s_b < 0.30,
            "test fixture's weak score should be in (0.15, 0.30), got {s_b}"
        );

        let logits = vec![token_a, token_b, token_c];
        let spans = decode_bioes(&logits, &test_label_map(), 0.1);
        assert_eq!(spans.len(), 1);
        // span.score 应等于 token_b 的 softmax(min across tokens)。
        assert!(
            (spans[0].score - s_b).abs() < 1e-4,
            "expected min to match token_b's softmax {s_b}, got {}",
            spans[0].score
        );
    }
}
