//! Privacy filter domain types — spans + 类目枚举 + 用户偏好。
//!
//! 见 docs/privacy-filter-spec.md §5.2 (Spans JSON schema) 和 §12
//! (Settings)。`PiiSpan` 是落盘 JSON 的 1:1 镜像；`PiiLabel` 收敛
//! OpenAI privacy-filter 的 8 个固定类目；`PrivacySettings` 是用户
//! 在 Settings 页配置的开关集合。
//!
//! 这一层**不**依赖 ort / tokenizers / 任何模型 runtime —— 仅是
//! 序列化形态。`services::privacy_filter` 负责模型加载、推理、缓存
//! 与 hook，本模块保持纯数据，方便 unit test 和 ts-rs export 单独走
//! 类型导出。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `services::privacy_filter::download::MODEL_MANIFEST` 加载后大约的
/// RSS 占用。q4f16 变体磁盘 ~830MB,加载到内存后 ORT 维持权重 + 中间
/// buffer 大约 1.5GB RSS(spec §4.3 / §10 估算)。Settings UI 用这个
/// 告诉用户开启后的内存成本;后续支持多 variant 时再改成 per-manifest
/// 计算。
pub const PRIVACY_MODEL_RAM_ESTIMATE_BYTES: u64 = 1_500_000_000;

/// Hugging Face 仓库 URL —— Settings UI 显示模型来源,可作为外链
/// 引导用户审阅模型许可、模型卡。
pub const PRIVACY_MODEL_SOURCE_URL: &str = "https://huggingface.co/openai/privacy-filter";

/// 模型整体状态快照,Settings UI 启动 + 刷新时读一次。`downloaded` 取
/// 决定整个 download flow 是否要触发;`total_size_bytes` /
/// `ram_estimate_bytes` 是用户开启前要被提醒的成本数字(spec §11.3
/// / §12.1)。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub struct PrivacyModelStatus {
    /// `true` 当 manifest 里所有文件都已落盘且(若有)sha256 校验过。
    pub downloaded: bool,
    /// 模型变体子目录名(`"privacy-filter-q4f16"`)—— 给 UI 用作 i18n
    /// 锚点 + 故障排查时的 label。
    pub variant: String,
    /// manifest 中所有文件大小之和(字节)。q4f16 ≈ 830MB。
    pub total_size_bytes: u64,
    /// 模型加载到内存后大约占的 RSS(字节)。q4f16 ≈ 1.5GB。
    pub ram_estimate_bytes: u64,
    /// 仓库主页 URL,Settings UI 显示成"模型来源:Hugging Face"链接。
    pub source_url: String,
}

/// 下载进度事件 —— `download_privacy_model` 通过 Tauri Channel 发给
/// Settings UI,UI 渲染进度条 + 当前在下哪个文件。
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub struct ModelDownloadProgress {
    /// 当前在下的文件名(`model_q4f16.onnx_data` 等)。
    pub file_name: String,
    pub downloaded_bytes: u64,
    /// 该文件期望总字节数。流式下载有时拿不到 Content-Length(`0`)。
    pub total_bytes: u64,
    /// 当前是第几个文件(1-indexed),让 UI 显示"2/4"。
    pub file_index: u32,
    /// manifest 里的文件总数。
    pub file_count: u32,
}

/// OpenAI privacy-filter 的 8 个固定类目。serde rename 锁定线协议为
/// snake_case，与模型 config.json 里的 `id2label` 字符串保持一致 ——
/// 之后从模型输出反序列化时不用做转换。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum PiiLabel {
    AccountNumber,
    PrivateAddress,
    PrivateEmail,
    PrivatePerson,
    PrivatePhone,
    PrivateUrl,
    PrivateDate,
    Secret,
}

impl PiiLabel {
    /// 给前端 UI / settings 用的中文显示名（spec §17 类目映射）。
    /// 不要依赖这个做内部比较 —— 内部一律走 enum 自身。
    pub fn display_name_zh(self) -> &'static str {
        match self {
            Self::AccountNumber => "账号",
            Self::PrivateAddress => "地址",
            Self::PrivateEmail => "邮箱",
            Self::PrivatePerson => "人名",
            Self::PrivatePhone => "电话",
            Self::PrivateUrl => "URL",
            Self::PrivateDate => "日期",
            Self::Secret => "密钥",
        }
    }

    /// Egress redact 模板。`secret` 走专属模板（已经在 capture 时
    /// 物理替换，这里只是兜底 —— 万一某个 secret span 没在 capture
    /// 时被替换，egress 仍能 catch）。
    pub fn redact_placeholder(self) -> &'static str {
        match self {
            Self::AccountNumber => "[账号]",
            Self::PrivateAddress => "[地址]",
            Self::PrivateEmail => "[邮箱]",
            Self::PrivatePerson => "[人名]",
            Self::PrivatePhone => "[电话]",
            Self::PrivateUrl => "[URL]",
            Self::PrivateDate => "[日期]",
            Self::Secret => "[REDACTED:secret]",
        }
    }
}

/// 一条 PII span。**v1600 起仅在内存中存在**：classify 输出 →
/// `services::privacy_filter::cache` LRU 缓存 → `redact::redact`
/// 消费。不再落盘到任何表(v1500 曾持久化到 `frames.ax_text_pii_spans`,
/// v1600 撤销)。结构仍然 derive Serialize/Deserialize/TS,前端如果
/// 将来需要在 UI 里高亮 redact 区段还会用到。
///
/// **`start` / `end` 是 char offset** —— 不是 byte offset。这样前端
/// highlight 不会把中文字符切成半个，Rust 端用 `text.chars().nth(...)`
/// 转换。spec §5.2 / §15 第 2 条。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub struct PiiSpan {
    pub start: usize,
    pub end: usize,
    pub label: PiiLabel,
    pub score: f32,
    /// `true` 仅当 label == Secret 且原文已被物理替换（capture 时
    /// 硬 redact，不可恢复）。其他情况下都是 `false`。
    /// 序列化时省略 false 值，让 JSON 小一点。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub redacted_in_storage: bool,
}

/// 用户在 Settings 页配置的隐私偏好。持久化到 `Config.privacy_filter`
/// (下一阶段在 domain/config.rs 接入)；UI 默认值见 spec §17。
///
/// `enabled = false` 时整个 pipeline 短路：capture 时不跑模型、egress
/// 时不做替换。`enabled = true` 但 `categories` 全 false 时，模型仍然
/// 跑（spans 入库可供未来使用），但 egress 不替换任何内容。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(default)]
pub struct PrivacySettings {
    /// 总开关。默认 `false` —— 用户必须同意下载模型后才能开启
    /// (spec §11.3)。
    pub enabled: bool,
    /// 每个类目独立开关。spec §17：URL / date 默认关，其他默认开，
    /// `secret` 强制开（UI 不允许关）。
    pub categories: CategoryToggles,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            categories: CategoryToggles::default(),
        }
    }
}

/// 8 个类目的独立开关。和 PiiLabel 一一对应 —— 加新 enum variant
/// 时这里也要加字段，编译器会提醒。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(default)]
pub struct CategoryToggles {
    pub account_number: bool,
    pub private_address: bool,
    pub private_email: bool,
    pub private_person: bool,
    pub private_phone: bool,
    pub private_url: bool,
    pub private_date: bool,
    /// Secret 在 schema 上允许 false，但在 settings.rs::write 路径里
    /// 会被强制成 true —— spec §12.1 / §17。
    pub secret: bool,
}

impl Default for CategoryToggles {
    fn default() -> Self {
        // spec §17 默认值。URL / date 误报多默认关；其他类目默认开。
        Self {
            account_number: true,
            private_address: true,
            private_email: true,
            private_person: true,
            private_phone: true,
            private_url: false,
            private_date: false,
            secret: true,
        }
    }
}

impl CategoryToggles {
    /// 查表：当前类目是否启用 egress redact。`secret` 总返回 true
    /// 因为该类目在 capture 时已物理替换；egress 时也照样替换以兜底。
    pub fn is_enabled(&self, label: PiiLabel) -> bool {
        match label {
            PiiLabel::AccountNumber => self.account_number,
            PiiLabel::PrivateAddress => self.private_address,
            PiiLabel::PrivateEmail => self.private_email,
            PiiLabel::PrivatePerson => self.private_person,
            PiiLabel::PrivatePhone => self.private_phone,
            PiiLabel::PrivateUrl => self.private_url,
            PiiLabel::PrivateDate => self.private_date,
            PiiLabel::Secret => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pii_label_serializes_snake_case() {
        // 验证 serde 形态对齐 OpenAI privacy-filter 的 id2label，
        // 这样模型输出可以直接反序列化进 PiiLabel。
        let json = serde_json::to_string(&PiiLabel::PrivatePerson).unwrap();
        assert_eq!(json, "\"private_person\"");

        let parsed: PiiLabel = serde_json::from_str("\"account_number\"").unwrap();
        assert_eq!(parsed, PiiLabel::AccountNumber);
    }

    #[test]
    fn pii_span_omits_false_redacted_flag() {
        // redacted_in_storage = false 时 JSON 里不出现该字段，
        // 落盘体积更小（大多数 span 都不是 secret）。
        let span = PiiSpan {
            start: 4,
            end: 14,
            label: PiiLabel::PrivatePerson,
            score: 0.998,
            redacted_in_storage: false,
        };
        let json = serde_json::to_string(&span).unwrap();
        assert!(!json.contains("redacted_in_storage"));
    }

    #[test]
    fn pii_span_includes_true_redacted_flag() {
        let span = PiiSpan {
            start: 0,
            end: 10,
            label: PiiLabel::Secret,
            score: 0.99,
            redacted_in_storage: true,
        };
        let json = serde_json::to_string(&span).unwrap();
        assert!(json.contains("\"redacted_in_storage\":true"));
    }

    #[test]
    fn default_settings_match_spec_table() {
        let s = PrivacySettings::default();
        // 总开关默认关 —— 用户同意下载后才能开
        assert!(!s.enabled);
        // 类目默认值表（spec §17）
        assert!(s.categories.private_person);
        assert!(s.categories.private_email);
        assert!(!s.categories.private_url);
        assert!(!s.categories.private_date);
        assert!(s.categories.secret);
    }

    #[test]
    fn secret_always_enabled_even_if_toggle_false() {
        // 即使有人手动构造一个 `secret: false`，is_enabled 必须返回
        // true —— UI 层不让关，逻辑层兜底。
        let mut toggles = CategoryToggles::default();
        toggles.secret = false;
        assert!(toggles.is_enabled(PiiLabel::Secret));
    }
}
