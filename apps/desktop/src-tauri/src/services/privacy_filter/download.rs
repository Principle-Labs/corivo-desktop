//! 模型 artifact 首启下载器 —— q4f16 ONNX + tokenizer + config。
//!
//! 设计目标（spec §11）:
//! - **不打包进 .dmg**：q4f16 总体积 ~830MB，进 bundle 不现实
//! - **流式下载 + 进度回调**：UI 能显示百分比
//! - **断点续传**：HTTP Range 请求，让弱网用户能恢复
//! - **sha256 校验**：防 CDN / 中间人投毒
//! - **幂等**：再次启动时跳过已就绪的文件
//!
//! 当前实装范围（这一轮）:
//! - `ModelManifest` 常量描述 q4f16 的 4 个文件 + URL + 期望大小
//! - `model_dir()` 解析目标目录到 `$APP_DATA/models/privacy-filter-q4f16/`
//! - `is_ready()` 判断是否所有文件都已下载且 sha256 校验通过
//! - `download_file()` 单文件下载 + 进度回调 + Range 续传 + sha 校验
//! - `ensure_downloaded()` 顶层入口：缺啥下啥
//!
//! 暂未实装:
//! - Tauri command 暴露 + Settings UI 联动 —— 下一轮 commands 接入
//!   时统一加。
//!
//! 已实装的 sha256 + commit pin:每个 ModelFile 都填实了
//! [`MODEL_MANIFEST`] 里的 `sha256` + 精确 `size_bytes`,URL 锁到
//! HF commit `7ffa9a0...385b`。HF 那边以后改 main 分支不会影响我们,
//! 模型升级要走显式 manifest bump。

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{CorivoError, Result};

/// 单个模型文件的描述。
#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    /// 在 model_dir 里的文件名（同时也是 HF repo 内的相对路径片段）
    pub name: &'static str,
    /// HF resolve URL,锁到具体 commit revision(不是 `main` 分支),
    /// 防 HF 那边后续改动让校验和漂移。
    pub url: &'static str,
    /// 期望字节数。HF 文件元信息已知,硬编码省一次 HEAD 请求。
    pub size_bytes: u64,
    /// 期望 sha256(小写 hex)。`None` 时跳过校验并 emit warning
    /// (spec §11.2 要求实质性校验)。当前所有文件都已填实。
    pub sha256: Option<&'static str>,
}

/// 一组属于同一变体（q4f16 / fp16 / ...）的文件清单。
#[derive(Debug, Clone, Copy)]
pub struct ModelManifest {
    /// 子目录名 ——`$APP_DATA/models/{variant_dir}/`
    pub variant_dir: &'static str,
    pub files: &'static [ModelFile],
}

/// **当前活动的 manifest**:OpenAI privacy-filter q4f16(block-quantized
/// + fp16 scale)变体 —— HF 上最小的 OnNX 变体,~830MB 磁盘。
///
/// 文件清单来自 https://huggingface.co/openai/privacy-filter ,锁到
/// commit `7ffa9a043d54d1be65afb281eddf0ffbe629385b` —— HF 那边以后
/// 改 main 分支不会让我们的 sha256 漂移。模型升级走 manifest bump,
/// 同时 `PrivacyFilter::clear_cache()` 要被调一次(避免旧 spans 跟新
/// 模型解码不一致)。
///
/// 文件构成:
/// - `onnx/model_q4f16.onnx`     : 162 KB(graph)
/// - `onnx/model_q4f16.onnx_data`: 772 MB(权重 blob)
/// - `tokenizer.json`            : 27 MB tokenizer (HF 标准位置在 repo 根)
/// - `config.json`               : 3 KB,含 id2label,decode.rs::LabelMap 用
///
/// **ort 版本锁定**:Cargo.toml 里 `ort = "=2.0.0-rc.12"`。q4f16 用的
/// `GatherBlockQuantized` (com.microsoft `bits` attribute) op 在
/// rc.10 bundled 的 onnxruntime 里**不支持**(报 "Unrecognized
/// attribute: bits"),rc.12 才修复。降级 ort 前要换 manifest 到
/// `model_quantized` (INT8,1.5GB,标准 contrib op) 作为兜底。
///
/// HF 同一仓库还有更大变体:`model_fp16` (2.8GB)、`model` (5.6GB fp32)。
/// 真要换变体时,size_bytes / sha256 都要重新算。
pub const MODEL_MANIFEST: ModelManifest = ModelManifest {
    variant_dir: "privacy-filter-q4f16",
    files: &[
        ModelFile {
            name: "model_q4f16.onnx",
            url: "https://huggingface.co/openai/privacy-filter/resolve/7ffa9a043d54d1be65afb281eddf0ffbe629385b/onnx/model_q4f16.onnx",
            size_bytes: 165_744,
            sha256: Some("eaae4e83cf1345a60abe333ed882b55fe5775d1dfbf34b9b269e5e5416f45e5b"),
        },
        ModelFile {
            name: "model_q4f16.onnx_data",
            url: "https://huggingface.co/openai/privacy-filter/resolve/7ffa9a043d54d1be65afb281eddf0ffbe629385b/onnx/model_q4f16.onnx_data",
            size_bytes: 809_061_992,
            sha256: Some("6d4dde787e03ace283c45d4e32a94eec32b6cfcc242e7219bea96f5b4c13569d"),
        },
        ModelFile {
            name: "tokenizer.json",
            url: "https://huggingface.co/openai/privacy-filter/resolve/7ffa9a043d54d1be65afb281eddf0ffbe629385b/tokenizer.json",
            size_bytes: 27_868_174,
            sha256: Some("0614fe83cadab421296e664e1f48f4261fa8fef6e03e63bb75c20f38e37d07d3"),
        },
        ModelFile {
            name: "config.json",
            url: "https://huggingface.co/openai/privacy-filter/resolve/7ffa9a043d54d1be65afb281eddf0ffbe629385b/config.json",
            size_bytes: 3_039,
            sha256: Some("b2b26a4a4a000639ad30b0c264adbefe365bdb567fbd7bb27303b8c438375bd1"),
        },
    ],
};

impl ModelManifest {
    /// 把 manifest 里所有文件的期望大小加起来。`size_bytes == 0`
    /// 的占位项不参与累加(目前 tokenizer.json / config.json 是这种
    /// 状态) —— 后续手动 dry-run 下载后填入精确值即可,UI 显示自然
    /// 变准。
    pub fn total_size_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size_bytes).sum()
    }

    pub fn file_count(&self) -> u32 {
        self.files.len() as u32
    }
}

/// 删掉模型目录(以及其中所有文件)。用于 Settings UI 的"重新下载"
/// 与"删除模型"按钮。删除不影响 PrivacyFilter 的内存状态;调用方负责
/// 同步关闭 settings.enabled,避免出现"开关 on 但模型不存在"。
pub async fn delete_model_dir(base: &Path, manifest: &ModelManifest) -> Result<()> {
    let dir = model_dir(base, manifest);
    match tokio::fs::remove_dir_all(&dir).await {
        Ok(_) => Ok(()),
        // 不存在视为"已经满足"
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(CorivoError::Internal(format!(
            "remove {} failed: {e}",
            dir.display()
        ))),
    }
}

/// `$APP_DATA/models/{variant_dir}/` 的完整路径。
///
/// 调用方拿 `AppHandle::path().app_data_dir()?` 当 base,把它喂进来。
/// 抽出 `base` 参数让单元测试能用 tempdir。
pub fn model_dir(base: &Path, manifest: &ModelManifest) -> PathBuf {
    base.join("models").join(manifest.variant_dir)
}

/// 检查 manifest 里所有文件是否已就绪。
///
/// "就绪" 的判定:
/// - 文件存在
/// - 大小匹配 `size_bytes`(若 manifest 里写了非零值)
/// - sha256 匹配(若 manifest 里写了)
///
/// 校验失败的文件**不会**被自动删除 —— 留给 caller 决定。
/// `download_file` 在续传前会自己处理。
pub async fn is_ready(base: &Path, manifest: &ModelManifest) -> bool {
    let dir = model_dir(base, manifest);
    for file in manifest.files {
        if !is_file_ready(&dir.join(file.name), file).await {
            return false;
        }
    }
    true
}

async fn is_file_ready(path: &Path, file: &ModelFile) -> bool {
    let Ok(meta) = tokio::fs::metadata(path).await else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    if file.size_bytes > 0 && meta.len() != file.size_bytes {
        return false;
    }
    if let Some(expected) = file.sha256 {
        match sha256_of_file(path).await {
            Ok(got) if got.eq_ignore_ascii_case(expected) => true,
            _ => false,
        }
    } else {
        // 没有 sha256 时,文件存在 + 大小匹配就当 ready —— 见模块级 TODO
        true
    }
}

/// 计算指定文件的 sha256(hex)。
pub async fn sha256_of_file(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| {
        CorivoError::Internal(format!("sha256: open {} failed: {e}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await.map_err(|e| {
            CorivoError::Internal(format!("sha256: read {} failed: {e}", path.display()))
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 下载进度回调形态。`(file_name, downloaded, total)` —— total 为 0
/// 表示服务器没给 Content-Length。
pub type ProgressCallback = std::sync::Arc<dyn Fn(&str, u64, u64) + Send + Sync>;

/// 单文件下载 + 续传 + sha 校验。
///
/// 流程:
/// 1. 如果 dest 已存在且 ready,直接返回 Ok
/// 2. 如果存在但不 ready —— 看大小:
///    - 大小 ≤ 期望: 发 Range request 续传
///    - 大小 > 期望: 删了重下(可能是上次下了错误的内容)
/// 3. 流式写盘 + 进度回调
/// 4. 下完校验 sha256
pub async fn download_file(
    file: &ModelFile,
    dest: &Path,
    progress: Option<ProgressCallback>,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            CorivoError::Internal(format!("mkdir {} failed: {e}", parent.display()))
        })?;
    }

    // 已就绪 —— 跳过
    if is_file_ready(dest, file).await {
        return Ok(());
    }

    // 决定 resume offset
    let resume_from = match tokio::fs::metadata(dest).await {
        Ok(meta) if meta.is_file() => {
            if file.size_bytes > 0 && meta.len() > file.size_bytes {
                // 比期望还大,八成是错的,删了重下
                tokio::fs::remove_file(dest).await.ok();
                0
            } else {
                meta.len()
            }
        }
        _ => 0,
    };

    let client = reqwest::Client::builder()
        // q4f16 onnx_data 接近 1GB; 大段下载需要长超时
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| CorivoError::Internal(format!("build reqwest client failed: {e}")))?;

    let mut req = client.get(file.url);
    if resume_from > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| CorivoError::Internal(format!("GET {} failed: {e}", file.url)))?;

    let status = resp.status();
    if !(status.is_success() || status == reqwest::StatusCode::PARTIAL_CONTENT) {
        return Err(CorivoError::Internal(format!(
            "GET {} returned HTTP {}",
            file.url, status
        )));
    }

    let content_length = resp.content_length().unwrap_or(0);
    let total = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        content_length + resume_from
    } else if file.size_bytes > 0 {
        file.size_bytes
    } else {
        content_length
    };

    // 打开文件:续传用 append,新下用 truncate
    let mut writer = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(resume_from == 0)
        .append(resume_from > 0)
        .open(dest)
        .await
        .map_err(|e| CorivoError::Internal(format!("open {} failed: {e}", dest.display())))?;

    let mut downloaded = resume_from;
    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|e| CorivoError::Internal(format!("download stream error: {e}")))?;
        writer
            .write_all(&chunk)
            .await
            .map_err(|e| CorivoError::Internal(format!("write {} failed: {e}", dest.display())))?;
        downloaded += chunk.len() as u64;
        if let Some(cb) = progress.as_ref() {
            cb(file.name, downloaded, total);
        }
    }
    writer.flush().await.ok();
    drop(writer);

    // sha256 校验
    if let Some(expected) = file.sha256 {
        let got = sha256_of_file(dest).await?;
        if !got.eq_ignore_ascii_case(expected) {
            // 校验失败 —— 删文件 + 报错。下次启动会重新下载。
            tokio::fs::remove_file(dest).await.ok();
            return Err(CorivoError::Internal(format!(
                "sha256 mismatch for {}: expected {expected}, got {got}",
                file.name
            )));
        }
    } else {
        tracing::warn!(
            file = file.name,
            "privacy_filter.download: sha256 unspecified in MODEL_MANIFEST; skipping verification"
        );
    }

    Ok(())
}

/// 顶层入口:确保 manifest 里所有文件都已下载。逐个文件串行下,
/// 因为 HF 端我们不想并发拉同一个 repo（rate limit）+ 用户也只
/// 想看一个进度条。
pub async fn ensure_downloaded(
    base: &Path,
    manifest: &ModelManifest,
    progress: Option<ProgressCallback>,
) -> Result<()> {
    let dir = model_dir(base, manifest);
    for file in manifest.files {
        let dest = dir.join(file.name);
        download_file(file, &dest, progress.clone()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    /// 期望 sha256 跟实际内容对得上时,is_file_ready 返回 true。
    /// 不依赖网络。
    #[tokio::test]
    async fn is_file_ready_passes_with_matching_sha() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, b"hello world").await.unwrap();
        // sha256("hello world") = b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
        let file = ModelFile {
            name: "a.bin",
            url: "",
            size_bytes: 11,
            sha256: Some("b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"),
        };
        assert!(is_file_ready(&path, &file).await);
    }

    #[tokio::test]
    async fn is_file_ready_fails_with_wrong_sha() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, b"hello world").await.unwrap();
        let file = ModelFile {
            name: "a.bin",
            url: "",
            size_bytes: 11,
            sha256: Some("0000000000000000000000000000000000000000000000000000000000000000"),
        };
        assert!(!is_file_ready(&path, &file).await);
    }

    #[tokio::test]
    async fn is_file_ready_fails_with_wrong_size() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, b"hello world").await.unwrap();
        let file = ModelFile {
            name: "a.bin",
            url: "",
            size_bytes: 999, // 实际是 11
            sha256: None,
        };
        assert!(!is_file_ready(&path, &file).await);
    }

    #[tokio::test]
    async fn is_file_ready_passes_when_sha_omitted_and_size_matches() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, b"hello world").await.unwrap();
        let file = ModelFile {
            name: "a.bin",
            url: "",
            size_bytes: 11,
            sha256: None,
        };
        assert!(is_file_ready(&path, &file).await);
    }

    #[tokio::test]
    async fn is_file_ready_fails_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing.bin");
        let file = ModelFile {
            name: "missing.bin",
            url: "",
            size_bytes: 11,
            sha256: None,
        };
        assert!(!is_file_ready(&path, &file).await);
    }

    #[tokio::test]
    async fn sha256_of_file_matches_known_value() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.bin");
        tokio::fs::write(&path, b"hello world").await.unwrap();
        let got = sha256_of_file(&path).await.unwrap();
        assert_eq!(
            got,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn model_dir_composes_under_models_subdir() {
        let base = Path::new("/tmp/corivo-test");
        let dir = model_dir(base, &MODEL_MANIFEST);
        assert!(dir.ends_with("models/privacy-filter-q4f16"));
        assert!(dir.starts_with("/tmp/corivo-test"));
    }

    #[test]
    fn model_manifest_files_are_unique_and_non_empty() {
        // 防御:别两个 ModelFile.name 重复 —— 它们会写到同一个路径。
        let names: Vec<&str> = MODEL_MANIFEST.files.iter().map(|f| f.name).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            names.len(),
            sorted.len(),
            "MODEL_MANIFEST has duplicate file names"
        );
        assert!(!MODEL_MANIFEST.files.is_empty());
    }

    /// 烟雾测试:progress callback 类型可以塞进 Option,clone,跨线程。
    #[test]
    fn progress_callback_is_clonable_and_send() {
        let cb: ProgressCallback = Arc::new(|_name, _done, _total| {});
        let opt = Some(cb);
        let _cloned = opt.clone();
    }
}
