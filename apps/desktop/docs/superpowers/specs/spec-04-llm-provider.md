# spec-04-llm-provider.md

## 一、目标

补全 LlmProvider trait 的完整方法集，并完成 GeminiProvider 的生产级实现。完成本 spec 后：

LlmProvider trait 包含三个方法：summarize_images（多图叙事总结，慢流水线用）、extract_structured（单图 JSON 抽取，快流水线预留）、health_check（spec-02 已有）
GeminiProvider 完整实现这三个方法，能真实调用 Gemini 2.0 Flash
准确的 token 计数和 cost 计算（基于 Gemini 真实定价表）
完善的错误处理：限流重试、超时、网络错误、配额超限
一个 MockLlmProvider 用于开发和测试，不消耗真实配额
LlmService 业务层封装，支持配置热重载
Tauri command：test_summary_with_images，让用户在配置页能用真实截图测试 prompt 效果

关键约束：和 MemoryProvider 一样，所有 Gemini 特有的概念（safetySettings、generationConfig 字段名、responseMimeType）必须封装在 GeminiProvider 内部。trait 层暴露的是通用 LLM 调用接口。未来切到 Claude Vision / GPT-4V 时业务代码不动。

## 二、不做什么

- ❌ 不实现 SummaryService 业务流程（spec-07 做）
- ❌ 不实现快流水线 SignalService（P1）
- ❌ 不接入截图采集流程（spec-06 做）
- ❌ 不做模型的本地部署（不集成 Ollama / LM Studio）
- ❌ 不做 streaming 响应（P0 一次性返回完整文本即可）
- ❌ 不做多模型自动 fallback（一个 provider 配置一个模型）
- ❌ 不做 prompt 缓存（Gemini 有 context cache 能力，P0 不用）

## 三、成功标准

完成本 spec 后：

cargo test 通过所有单元测试
配置真实 Gemini key 后，cargo test --test integration_gemini -- --ignored 跑通真实 API 集成
调用 summarize_images 传入 5 张测试截图 + 一段 prompt，能拿到合理的中文总结文本
返回的 LlmResponse 包含准确的 input_tokens / output_tokens / cost_usd
当 API 返回 429 时自动重试，最多 2 次，指数退避
当配额用尽时返回明确的 QuotaExceeded 错误，不是模糊的 HTTP 错误
配置页能用真实截图测试 prompt 效果，验证 prompt 调优体验

## 四、LlmProvider trait 完整定义

src-tauri/src/providers/llm/mod.rs：
rustuse async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::error::Result;

pub mod gemini;
pub mod mock;
pub mod types;

pub use types::*;

#[async_trait]
pub trait LlmProvider: Send + Sync {
/// 健康检查（spec-02 已有）
async fn health_check(&self) -> Result<()>;

    /// 多图叙事总结：把一组按时间顺序的图片 + prompt 发给模型，拿到自然语言总结。
    /// 慢流水线 SummaryService 主用此方法。
    async fn summarize_images(
        &self,
        images: Vec<ImageInput>,
        prompt: String,
    ) -> Result<LlmResponse>;
    
    /// 单图结构化抽取：从一张图片里抽取符合 JSON schema 的结构化数据。
    /// P0 不调用，P1 快流水线 SignalService 使用。但 P0 必须实现完整可用，
    /// 因为这是 trait 契约的一部分，且能用于配置页 prompt 调试。
    async fn extract_structured(
        &self,
        image: ImageInput,
        json_schema: serde_json::Value,
        prompt: String,
    ) -> Result<StructuredResponse>;
    
    /// 模型描述符
    fn model_info(&self) -> ModelInfo;
}

## 五、通用数据类型

src-tauri/src/providers/llm/types.rs：
rustuse serde::{Deserialize, Serialize};

/// 图片输入
#[derive(Debug, Clone)]
pub struct ImageInput {
pub mime_type: String,  // "image/jpeg" / "image/png"
pub data: Vec<u8>,
}

impl ImageInput {
pub fn jpeg(data: Vec<u8>) -> Self {
Self {
mime_type: "image/jpeg".to_string(),
data,
}
}
}

/// 总结调用的响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
pub text: String,
pub input_tokens: u32,
pub output_tokens: u32,
pub cost_usd: f64,
pub model: String,
pub finish_reason: FinishReason,
}

/// 结构化抽取的响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredResponse {
pub data: serde_json::Value,
pub input_tokens: u32,
pub output_tokens: u32,
pub cost_usd: f64,
pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
Stop,            // 正常结束
MaxTokens,       // 达到最大 token 数
Safety,          // 被安全策略截断
Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
pub provider: String,           // "gemini"
pub model: String,              // "gemini-2.0-flash"
pub supports_vision: bool,
pub supports_structured_output: bool,
/// 每百万 input token 的价格 (USD)
pub input_price_per_1m: f64,
/// 每百万 output token 的价格 (USD)
pub output_price_per_1m: f64,
}

## 六、定价表（关键）

不同模型的定价封装成常量，未来加新模型只改这一处。
src-tauri/src/providers/llm/pricing.rs：
rust//! Gemini 模型定价表（截至 2026 年 4 月）
//! 实际生效价格以 Google 官方文档为准: https://ai.google.dev/pricing

pub struct ModelPricing {
pub input_per_1m: f64,
pub output_per_1m: f64,
}

pub fn pricing_for(model: &str) -> ModelPricing {
match model {
"gemini-2.0-flash" | "gemini-2.0-flash-001" => ModelPricing {
input_per_1m: 0.10,   // 文本 + 图片输入
output_per_1m: 0.40,
},
"gemini-2.0-flash-lite" => ModelPricing {
input_per_1m: 0.075,
output_per_1m: 0.30,
},
"gemini-1.5-flash" => ModelPricing {
input_per_1m: 0.075,
output_per_1m: 0.30,
},
"gemini-1.5-pro" => ModelPricing {
input_per_1m: 1.25,
output_per_1m: 5.00,
},
_ => {
tracing::warn!("unknown model {}, using default pricing", model);
ModelPricing {
input_per_1m: 0.10,
output_per_1m: 0.40,
}
}
}
}

pub fn calculate_cost(model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
let p = pricing_for(model);
let input_cost = (input_tokens as f64) * p.input_per_1m / 1_000_000.0;
let output_cost = (output_tokens as f64) * p.output_per_1m / 1_000_000.0;
input_cost + output_cost
}

#[cfg(test)]
mod tests {
use super::*;

    #[test]
    fn test_cost_calculation() {
        // 100k input + 1k output for flash
        let cost = calculate_cost("gemini-2.0-flash", 100_000, 1_000);
        // 100_000 * 0.10 / 1M + 1_000 * 0.40 / 1M = 0.01 + 0.0004 = 0.0104
        assert!((cost - 0.0104).abs() < 0.00001);
    }
    
    #[test]
    fn test_unknown_model_fallback() {
        let p = pricing_for("nonexistent-model");
        assert_eq!(p.input_per_1m, 0.10);
    }
}

## 七、错误类型扩展

src-tauri/src/error.rs 增加 LLM 相关变体：
rust#[derive(Debug, Error)]
pub enum CorivoError {
// ... 已有变体

    #[error("LLM error: {0}")]
    Llm(String),
    
    #[error("LLM quota exceeded: {0}")]
    QuotaExceeded(String),
    
    #[error("LLM safety blocked: {0}")]
    SafetyBlocked(String),
    
    #[error("LLM invalid response: {0}")]
    InvalidResponse(String),
}

## 八、GeminiProvider 完整实现

### 8.1 整体结构

src-tauri/src/providers/llm/gemini.rs：
rustuse super::pricing::calculate_cost;
use super::{
FinishReason, ImageInput, LlmProvider, LlmResponse, ModelInfo, StructuredResponse,
};
use crate::error::{CorivoError, Result};
use async_trait::async_trait;
use base64::Engine;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const DEFAULT_MODEL: &str = "gemini-2.0-flash";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_RETRIES: u32 = 2;
const MAX_OUTPUT_TOKENS: u32 = 2048;

pub struct GeminiProvider {
api_key: String,
model: String,
client: Client,
base_url: String,
}

impl GeminiProvider {
pub fn new(api_key: String) -> Result<Self> {
Self::with_model(api_key, DEFAULT_MODEL.to_string())
}

    pub fn with_model(api_key: String, model: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .map_err(|e| CorivoError::Network(e.to_string()))?;
        
        Ok(Self {
            api_key,
            model,
            client,
            base_url: GEMINI_BASE_URL.to_string(),
        })
    }
    
    fn endpoint(&self, action: &str) -> String {
        format!(
            "{}/models/{}:{}?key={}",
            self.base_url, self.model, action, self.api_key
        )
    }
    
    /// 把图片编码成 Gemini 接受的 inline_data 格式
    fn image_to_part(image: &ImageInput) -> GeminiPart {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&image.data);
        GeminiPart::InlineData {
            inline_data: GeminiInlineData {
                mime_type: image.mime_type.clone(),
                data: b64,
            },
        }
    }
    
    /// 通用执行 + 重试 + 错误映射
    async fn execute_with_retry<F, Fut>(&self, op: F) -> Result<reqwest::Response>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = std::result::Result<reqwest::Response, reqwest::Error>>,
    {
        let mut attempts = 0u32;
        loop {
            match op().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status == StatusCode::TOO_MANY_REQUESTS && attempts < MAX_RETRIES {
                        attempts += 1;
                        let backoff = Duration::from_millis(1000 * 2u64.pow(attempts));
                        tracing::warn!(
                            "Gemini rate limited, retry {}/{} after {:?}",
                            attempts,
                            MAX_RETRIES,
                            backoff
                        );
                        sleep(backoff).await;
                        continue;
                    }
                    return Ok(resp);
                }
                Err(e) if attempts < MAX_RETRIES && e.is_timeout() => {
                    attempts += 1;
                    let backoff = Duration::from_millis(1000 * 2u64.pow(attempts));
                    tracing::warn!("Gemini timeout, retry after {:?}", backoff);
                    sleep(backoff).await;
                }
                Err(e) => return Err(CorivoError::Network(e.to_string())),
            }
        }
    }
    
    fn map_error_response(&self, status: StatusCode, body: &str) -> CorivoError {
        // Gemini 错误通常是 JSON: { "error": { "code": ..., "message": ..., "status": ... } }
        let parsed: Option<GeminiErrorBody> = serde_json::from_str(body).ok();
        let message = parsed
            .as_ref()
            .map(|e| e.error.message.clone())
            .unwrap_or_else(|| body.to_string());
        let api_status = parsed
            .as_ref()
            .map(|e| e.error.status.clone())
            .unwrap_or_default();
        
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                CorivoError::Llm(format!("Gemini: API key 无效或无权限 - {}", message))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                CorivoError::QuotaExceeded(format!("Gemini: {}", message))
            }
            StatusCode::BAD_REQUEST => {
                if api_status == "RESOURCE_EXHAUSTED" {
                    CorivoError::QuotaExceeded(format!("Gemini: {}", message))
                } else {
                    CorivoError::Llm(format!("Gemini bad request: {}", message))
                }
            }
            _ => CorivoError::Llm(format!("Gemini HTTP {}: {}", status, message)),
        }
    }
    
    fn parse_summary_response(&self, body: GeminiGenerateResponse) -> Result<LlmResponse> {
        let candidate = body
            .candidates
            .into_iter()
            .next()
            .ok_or_else(|| CorivoError::InvalidResponse("Gemini 无候选输出".to_string()))?;
        
        let finish_reason = match candidate.finish_reason.as_deref() {
            Some("STOP") => FinishReason::Stop,
            Some("MAX_TOKENS") => FinishReason::MaxTokens,
            Some("SAFETY") => {
                return Err(CorivoError::SafetyBlocked(
                    "Gemini 内容被安全策略拦截".to_string(),
                ));
            }
            _ => FinishReason::Other,
        };
        
        let text = candidate
            .content
            .and_then(|c| {
                c.parts
                    .into_iter()
                    .filter_map(|p| match p {
                        GeminiPart::Text { text } => Some(text),
                        _ => None,
                    })
                    .next()
            })
            .unwrap_or_default();
        
        let usage = body.usage_metadata.unwrap_or_default();
        let cost = calculate_cost(&self.model, usage.prompt_token_count, usage.candidates_token_count);
        
        Ok(LlmResponse {
            text,
            input_tokens: usage.prompt_token_count,
            output_tokens: usage.candidates_token_count,
            cost_usd: cost,
            model: self.model.clone(),
            finish_reason,
        })
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
async fn health_check(&self) -> Result<()> {
let url = format!("{}/models?key={}", self.base_url, self.api_key);
let resp = self
.client
.get(&url)
.send()
.await
.map_err(|e| CorivoError::Network(e.to_string()))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(self.map_error_response(status, &body))
        }
    }
    
    async fn summarize_images(
        &self,
        images: Vec<ImageInput>,
        prompt: String,
    ) -> Result<LlmResponse> {
        if images.is_empty() {
            return Err(CorivoError::Llm("summarize_images 需要至少一张图片".to_string()));
        }
        
        // 构造 parts: 先 prompt 文本，再所有图片
        let mut parts = vec![GeminiPart::Text { text: prompt }];
        for img in &images {
            parts.push(Self::image_to_part(img));
        }
        
        let request = GeminiGenerateRequest {
            contents: vec![GeminiContent { parts }],
            generation_config: GeminiGenerationConfig {
                temperature: 0.3,
                max_output_tokens: MAX_OUTPUT_TOKENS,
                response_mime_type: None,
                response_schema: None,
            },
        };
        
        let url = self.endpoint("generateContent");
        
        let resp = self
            .execute_with_retry(|| async {
                self.client.post(&url).json(&request).send().await
            })
            .await?;
        
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(self.map_error_response(status, &body));
        }
        
        let body: GeminiGenerateResponse = resp
            .json()
            .await
            .map_err(|e| CorivoError::InvalidResponse(format!("解析 Gemini 响应失败: {}", e)))?;
        
        self.parse_summary_response(body)
    }
    
    async fn extract_structured(
        &self,
        image: ImageInput,
        json_schema: serde_json::Value,
        prompt: String,
    ) -> Result<StructuredResponse> {
        let parts = vec![
            GeminiPart::Text { text: prompt },
            Self::image_to_part(&image),
        ];
        
        let request = GeminiGenerateRequest {
            contents: vec![GeminiContent { parts }],
            generation_config: GeminiGenerationConfig {
                temperature: 0.1,
                max_output_tokens: MAX_OUTPUT_TOKENS,
                response_mime_type: Some("application/json".to_string()),
                response_schema: Some(json_schema),
            },
        };
        
        let url = self.endpoint("generateContent");
        
        let resp = self
            .execute_with_retry(|| async {
                self.client.post(&url).json(&request).send().await
            })
            .await?;
        
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(self.map_error_response(status, &body));
        }
        
        let body: GeminiGenerateResponse = resp
            .json()
            .await
            .map_err(|e| CorivoError::InvalidResponse(format!("解析 Gemini 响应失败: {}", e)))?;
        
        let candidate = body
            .candidates
            .into_iter()
            .next()
            .ok_or_else(|| CorivoError::InvalidResponse("Gemini 无候选输出".to_string()))?;
        
        let raw_text = candidate
            .content
            .and_then(|c| {
                c.parts
                    .into_iter()
                    .filter_map(|p| match p {
                        GeminiPart::Text { text } => Some(text),
                        _ => None,
                    })
                    .next()
            })
            .unwrap_or_default();
        
        // Gemini 偶尔会用 ```json ... ``` 包裹，做容错
        let cleaned = raw_text
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();
        
        let data: serde_json::Value = serde_json::from_str(cleaned).map_err(|e| {
            CorivoError::InvalidResponse(format!("结构化输出非合法 JSON: {} (raw: {})", e, raw_text))
        })?;
        
        let usage = body.usage_metadata.unwrap_or_default();
        let cost = calculate_cost(&self.model, usage.prompt_token_count, usage.candidates_token_count);
        
        Ok(StructuredResponse {
            data,
            input_tokens: usage.prompt_token_count,
            output_tokens: usage.candidates_token_count,
            cost_usd: cost,
            model: self.model.clone(),
        })
    }
    
    fn model_info(&self) -> ModelInfo {
        let p = super::pricing::pricing_for(&self.model);
        ModelInfo {
            provider: "gemini".to_string(),
            model: self.model.clone(),
            supports_vision: true,
            supports_structured_output: true,
            input_price_per_1m: p.input_per_1m,
            output_price_per_1m: p.output_per_1m,
        }
    }
}

// === Gemini wire types ===

#[derive(Debug, Serialize)]
struct GeminiGenerateRequest {
contents: Vec<GeminiContent>,
#[serde(rename = "generationConfig")]
generation_config: GeminiGenerationConfig,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiContent {
parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum GeminiPart {
Text { text: String },
InlineData {
#[serde(rename = "inlineData", alias = "inline_data")]
inline_data: GeminiInlineData,
},
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiInlineData {
#[serde(rename = "mimeType", alias = "mime_type")]
mime_type: String,
data: String,
}

#[derive(Debug, Serialize)]
struct GeminiGenerationConfig {
temperature: f32,
#[serde(rename = "maxOutputTokens")]
max_output_tokens: u32,
#[serde(rename = "responseMimeType", skip_serializing_if = "Option::is_none")]
response_mime_type: Option<String>,
#[serde(rename = "responseSchema", skip_serializing_if = "Option::is_none")]
response_schema: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GeminiGenerateResponse {
candidates: Vec<GeminiCandidate>,
#[serde(rename = "usageMetadata", default)]
usage_metadata: Option<GeminiUsage>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
content: Option<GeminiContent>,
#[serde(rename = "finishReason")]
finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct GeminiUsage {
#[serde(rename = "promptTokenCount", default)]
prompt_token_count: u32,
#[serde(rename = "candidatesTokenCount", default)]
candidates_token_count: u32,
}

#[derive(Debug, Deserialize)]
struct GeminiErrorBody {
error: GeminiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct GeminiErrorDetail {
message: String,
#[serde(default)]
status: String,
}
关键点解释：

GeminiPart 用 untagged enum：Gemini 的 parts 数组里既有 { "text": "..." } 也有 { "inline_data": {...} }，没有共享的 type 字段，必须 untagged。
responseSchema 是 Option：只有结构化抽取时才传，叙事总结时不传，避免不必要的约束。
错误响应解析做了两层兜底：先尝试解析成 GeminiErrorBody，失败就用原始 body 字符串。Gemini 偶尔会返回非 JSON 错误（比如 nginx 的 502 HTML），不能假设都是 JSON。
SAFETY finish_reason 单独处理：返回专门的 SafetyBlocked 错误而不是混进通用错误，这样未来 UI 可以针对性提示用户"内容包含敏感信息"。
图片放在 prompt 之后：Gemini 的 best practice 是 prompt 在前，图片在后，效果略好于反过来。

### 8.2 新增依赖

tomlbase64 = "0.22"

## 九、MockLlmProvider

src-tauri/src/providers/llm/mock.rs：
rustuse super::{
FinishReason, ImageInput, LlmProvider, LlmResponse, ModelInfo, StructuredResponse,
};
use crate::error::Result;
use async_trait::async_trait;
use std::sync::atomic::{AtomicU32, Ordering};

pub struct MockLlmProvider {
call_count: AtomicU32,
}

impl MockLlmProvider {
pub fn new() -> Self {
Self {
call_count: AtomicU32::new(0),
}
}

    pub fn call_count(&self) -> u32 {
        self.call_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
async fn health_check(&self) -> Result<()> {
Ok(())
}

    async fn summarize_images(
        &self,
        images: Vec<ImageInput>,
        _prompt: String,
    ) -> Result<LlmResponse> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        // 模拟一个合理的 token 用量
        let input_tokens = (images.len() as u32) * 258 + 100;
        let output_tokens = 80;
        Ok(LlmResponse {
            text: format!(
                "[Mock] 用户在过去的时间段内进行了 {} 张截图覆盖的活动，主要集中在编码和文档查阅。",
                images.len()
            ),
            input_tokens,
            output_tokens,
            cost_usd: 0.0,
            model: "mock-llm".to_string(),
            finish_reason: FinishReason::Stop,
        })
    }
    
    async fn extract_structured(
        &self,
        _image: ImageInput,
        _json_schema: serde_json::Value,
        _prompt: String,
    ) -> Result<StructuredResponse> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(StructuredResponse {
            data: serde_json::json!({
                "current_activity": "Mock activity",
                "facts": []
            }),
            input_tokens: 300,
            output_tokens: 50,
            cost_usd: 0.0,
            model: "mock-llm".to_string(),
        })
    }
    
    fn model_info(&self) -> ModelInfo {
        ModelInfo {
            provider: "mock".to_string(),
            model: "mock-llm".to_string(),
            supports_vision: true,
            supports_structured_output: true,
            input_price_per_1m: 0.0,
            output_price_per_1m: 0.0,
        }
    }
}
关键设计：Mock provider 的 call_count 让单元测试能断言"SummaryService 调用了几次 LLM"，对验证去重 / 重试逻辑很有用。

## 十、LlmService 业务层

src-tauri/src/services/llm_service.rs：
rustuse crate::error::Result;
use crate::providers::llm::{
gemini::GeminiProvider, mock::MockLlmProvider,
ImageInput, LlmProvider, LlmResponse, ModelInfo, StructuredResponse,
};
use crate::services::{
config_service::ConfigService,
keychain_service::{KeychainKey, KeychainService},
};
use std::sync::{Arc, RwLock};

pub struct LlmService {
provider: RwLock<Arc<dyn LlmProvider>>,
keychain: Arc<KeychainService>,
config: Arc<ConfigService>,
}

impl LlmService {
pub fn new(keychain: Arc<KeychainService>, config: Arc<ConfigService>) -> Self {
let provider = Self::build_provider(&keychain, &config);
Self {
provider: RwLock::new(provider),
keychain,
config,
}
}

    fn build_provider(
        keychain: &KeychainService,
        config: &ConfigService,
    ) -> Arc<dyn LlmProvider> {
        let model = config.get().summary.model;
        match keychain.load(KeychainKey::GeminiApiKey) {
            Ok(Some(key)) => match GeminiProvider::with_model(key, model) {
                Ok(p) => Arc::new(p),
                Err(e) => {
                    tracing::error!("failed to init GeminiProvider: {:?}, fallback to mock", e);
                    Arc::new(MockLlmProvider::new())
                }
            },
            _ => {
                tracing::warn!("Gemini API key not configured, using MockLlmProvider");
                Arc::new(MockLlmProvider::new())
            }
        }
    }
    
    pub fn reload(&self) {
        let new_provider = Self::build_provider(&self.keychain, &self.config);
        *self.provider.write().unwrap() = new_provider;
        tracing::info!("LlmService provider reloaded");
    }
    
    fn current(&self) -> Arc<dyn LlmProvider> {
        self.provider.read().unwrap().clone()
    }
    
    pub async fn summarize_images(
        &self,
        images: Vec<ImageInput>,
        prompt: String,
    ) -> Result<LlmResponse> {
        self.current().summarize_images(images, prompt).await
    }
    
    pub async fn extract_structured(
        &self,
        image: ImageInput,
        schema: serde_json::Value,
        prompt: String,
    ) -> Result<StructuredResponse> {
        self.current().extract_structured(image, schema, prompt).await
    }
    
    pub async fn health_check(&self) -> Result<()> {
        self.current().health_check().await
    }
    
    pub fn model_info(&self) -> ModelInfo {
        self.current().model_info()
    }
}

## 十一、Tauri Commands

src-tauri/src/commands/llm.rs（新增）：
rustuse crate::providers::llm::{ImageInput, LlmResponse, ModelInfo};
use crate::services::llm_service::LlmService;
use std::sync::Arc;
use tauri::State;

pub struct LlmAppState {
pub llm_service: Arc<LlmService>,
}

#[tauri::command]
pub async fn get_model_info(state: State<'_, LlmAppState>) -> Result<ModelInfo, String> {
Ok(state.llm_service.model_info())
}

/// 调试用：用真实截图测试 prompt 效果
/// images_base64 是一个 base64 编码的 JPEG 数组
#[tauri::command]
pub async fn test_summary_with_images(
state: State<'_, LlmAppState>,
images_base64: Vec<String>,
prompt: String,
) -> Result<LlmResponse, String> {
use base64::Engine;

    let images: std::result::Result<Vec<ImageInput>, String> = images_base64
        .into_iter()
        .map(|b64| {
            base64::engine::general_purpose::STANDARD
                .decode(&b64)
                .map(ImageInput::jpeg)
                .map_err(|e| format!("base64 decode failed: {}", e))
        })
        .collect();
    
    let images = images?;
    state
        .llm_service
        .summarize_images(images, prompt)
        .await
        .map_err(Into::into)
}
更新 lib.rs setup：
rustlet llm_service = Arc::new(LlmService::new(
keychain_service.clone(),
config_service.clone(),
));
app.manage(LlmAppState { llm_service: llm_service.clone() });

// invoke_handler 增加
commands::llm::get_model_info,
commands::llm::test_summary_with_images,
save_gemini_api_key 命令也需要增加 reload 钩子（类似 spec-03 的 supermemory）：
rust#[tauri::command]
pub async fn save_gemini_api_key(
state: State<'_, AppState>,
llm_state: State<'_, LlmAppState>,
api_key: String,
) -> Result<(), String> {
state
.keychain_service
.save(KeychainKey::GeminiApiKey, &api_key)
.map_err(String::from)?;
llm_state.llm_service.reload();
Ok(())
}

## 十二、前端类型与封装

src/lib/types.ts 增加：
tsexport type FinishReason = 'stop' | 'max_tokens' | 'safety' | 'other'

export interface LlmResponse {
text: string
input_tokens: number
output_tokens: number
cost_usd: number
model: string
finish_reason: FinishReason
}

export interface ModelInfo {
provider: string
model: string
supports_vision: boolean
supports_structured_output: boolean
input_price_per_1m: number
output_price_per_1m: number
}
src/lib/tauri.ts 增加：
tsimport type { LlmResponse, ModelInfo } from './types'

export async function getModelInfo(): Promise<ModelInfo> {
return invoke<ModelInfo>('get_model_info')
}

export async function testSummaryWithImages(
imagesBase64: string[],
prompt: string
): Promise<LlmResponse> {
return invoke<LlmResponse>('test_summary_with_images', {
imagesBase64,
prompt,
})
}

## 十三、测试

### 13.1 pricing 单元测试

已经在 pricing.rs 内部写了。

### 13.2 Mock 单元测试

src-tauri/src/providers/llm/mock.rs 末尾追加：
rust#[cfg(test)]
mod tests {
use super::*;

    #[tokio::test]
    async fn test_mock_summarize() {
        let p = MockLlmProvider::new();
        let images = vec![
            ImageInput::jpeg(vec![0u8; 100]),
            ImageInput::jpeg(vec![0u8; 100]),
        ];
        let resp = p.summarize_images(images, "test prompt".to_string()).await.unwrap();
        assert!(!resp.text.is_empty());
        assert_eq!(resp.input_tokens, 258 * 2 + 100);
        assert_eq!(p.call_count(), 1);
    }
    
    #[tokio::test]
    async fn test_mock_extract_structured() {
        let p = MockLlmProvider::new();
        let image = ImageInput::jpeg(vec![0u8; 100]);
        let resp = p
            .extract_structured(image, serde_json::json!({}), "test".to_string())
            .await
            .unwrap();
        assert!(resp.data.is_object());
    }
}

### 13.3 GeminiProvider 单元测试（用 wiremock）

新增 dev dependency：
toml[dev-dependencies]
wiremock = "0.6"
src-tauri/src/providers/llm/gemini.rs 末尾追加：
rust#[cfg(test)]
mod tests {
use super::*;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

    fn mock_success_response() -> serde_json::Value {
        serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{ "text": "测试总结：用户在编码。" }]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 1000,
                "candidatesTokenCount": 50
            }
        })
    }
    
    #[tokio::test]
    async fn test_summarize_images_success() {
        let server = MockServer::start().await;
        
        Mock::given(method("POST"))
            .and(path_regex(r".*generateContent.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_success_response()))
            .mount(&server)
            .await;
        
        let mut provider = GeminiProvider::new("test_key".to_string()).unwrap();
        provider.base_url = server.uri();
        
        let images = vec![ImageInput::jpeg(vec![0u8; 100])];
        let resp = provider
            .summarize_images(images, "test".to_string())
            .await
            .unwrap();
        
        assert_eq!(resp.text, "测试总结：用户在编码。");
        assert_eq!(resp.input_tokens, 1000);
        assert_eq!(resp.output_tokens, 50);
        assert_eq!(resp.finish_reason, FinishReason::Stop);
        assert!(resp.cost_usd > 0.0);
    }
    
    #[tokio::test]
    async fn test_429_retry() {
        let server = MockServer::start().await;
        
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_success_response()))
            .mount(&server)
            .await;
        
        let mut provider = GeminiProvider::new("test_key".to_string()).unwrap();
        provider.base_url = server.uri();
        
        let images = vec![ImageInput::jpeg(vec![0u8; 100])];
        let resp = provider.summarize_images(images, "test".to_string()).await;
        assert!(resp.is_ok());
    }
    
    #[tokio::test]
    async fn test_extract_structured_with_markdown_wrapper() {
        let server = MockServer::start().await;
        
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "candidates": [{
                    "content": {
                        "parts": [{ "text": "```json\n{\"key\": \"value\"}\n```" }]
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": { "promptTokenCount": 100, "candidatesTokenCount": 10 }
            })))
            .mount(&server)
            .await;
        
        let mut provider = GeminiProvider::new("test_key".to_string()).unwrap();
        provider.base_url = server.uri();
        
        let resp = provider
            .extract_structured(
                ImageInput::jpeg(vec![0u8; 100]),
                serde_json::json!({}),
                "test".to_string(),
            )
            .await
            .unwrap();
        
        assert_eq!(resp.data["key"], "value");
    }
}

### 13.4 真实 API 集成测试

src-tauri/tests/integration_gemini.rs：
rust//! 真实 Gemini API 集成测试
//! 跑法: GEMINI_API_KEY=xxx cargo test --test integration_gemini -- --ignored --nocapture

use corivo::providers::llm::{gemini::GeminiProvider, ImageInput, LlmProvider};

fn get_key() -> String {
std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY not set")
}

/// 一个最小的合法 JPEG（1x1 白色像素）
fn tiny_jpeg() -> Vec<u8> {
vec![
0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01,
0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43,
0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08, 0x07, 0x07, 0x07, 0x09,
// ... 实际编码时用一个真实的 1x1 JPEG 文件读取，这里省略
]
}

#[tokio::test]
#[ignore]
async fn test_health_check() {
let provider = GeminiProvider::new(get_key()).unwrap();
provider.health_check().await.expect("health check failed");
}

#[tokio::test]
#[ignore]
async fn test_summarize_real_image() {
let provider = GeminiProvider::new(get_key()).unwrap();

    // 实际测试请把 tests/fixtures/test_screenshot.jpg 放进去并读取
    let img_data = std::fs::read("tests/fixtures/test_screenshot.jpg")
        .expect("test fixture not found");
    
    let images = vec![ImageInput::jpeg(img_data)];
    let prompt = "请用一句话描述这张截图的内容。".to_string();
    
    let resp = provider.summarize_images(images, prompt).await.unwrap();
    println!("Gemini response: {}", resp.text);
    println!("Tokens: in={} out={}", resp.input_tokens, resp.output_tokens);
    println!("Cost: ${:.6}", resp.cost_usd);
    
    assert!(!resp.text.is_empty());
    assert!(resp.input_tokens > 0);
}
注意：你需要在 src-tauri/tests/fixtures/ 下放一张真实截图 test_screenshot.jpg 用于集成测试。

## 十四、任务分解

会话范围完成判定1types.rs + pricing.rs + 单元测试 + 错误类型扩展cargo test pricing 通过2MockLlmProvider 完整实现 + 单元测试cargo test mock 通过3GeminiProvider 完整实现（不含测试）cargo build 通过4wiremock 集成 + Gemini 单元测试cargo test gemini 通过5LlmService + commands/llm.rs + lib.rs 装配 + save_key reload 钩子启动 app 后能 invoke get_model_info6前端 types + tauri.ts 封装类型对齐无报错7tests/integration_gemini.rs + 真实 key 集成测试用真实 key 跑通

## 十五、验收清单

cargo test 全部通过
cargo clippy 无 warning
没配 Gemini key 时启动 app，调用 test_summary_with_images 走 Mock 返回假总结
配置正确 Gemini key 后，再次调用 test_summary_with_images 走真实 API 返回真实总结
真实 API 调用返回的 cost_usd 与手动计算一致
模拟 429 响应能触发重试，最终成功
模拟 401 响应返回明确的"API key 无效"错误
结构化抽取能正确处理 ```json 包裹的响应
cargo test --test integration_gemini -- --ignored 跑通
前端 import 'testSummaryWithImages' 类型提示正常

## 十六、坑点预警

responseSchema 的 Gemini 限制：Gemini 的结构化输出对 schema 复杂度有限制，深度嵌套和某些 OpenAPI 字段（如 oneOf）不支持。P1 写实际 schema 时可能踩坑。P0 不调用所以不暴雷。
图片大小限制：Gemini 单次请求总大小约 20MB。30 张 1440p JPEG 大约 10-15MB，刚好够。如果用 4K 截图可能超限，spec-06 截图采集时要做 resize 防御。
token 计数的"图片视为 258 tokens"：Gemini 文档说每张图片相当于 258 tokens（小图）或更多（高分辨率）。usageMetadata.promptTokenCount 是真实的，不要自己算。
timeout 60 秒可能不够：30 张图的 summarize 偶尔需要 30-50 秒。设置 60 秒 + 2 次重试应该够。如果实际跑下来还是超时，调大到 120 秒。
safety filter 误杀：Gemini 的 safety filter 偶尔会把正常截图判为敏感（特别是医疗、新闻类内容）。返回 SafetyBlocked 时不要重试，直接报错。
finish_reason = "MAX_TOKENS" 时 text 是被截断的：业务层（spec-07 SummaryService）需要决定是接受截断结果还是丢弃重试。本 spec 只如实返回。
base64 编码的内存峰值：30 张 JPEG 总共 15MB → base64 后约 20MB → 整个 request body 约 25MB。Rust 内存够，但要注意不要在 hot path 反复 clone。
untagged enum 的反序列化：GeminiPart 用 untagged 容易报模糊错误，wiremock 测试要覆盖这部分确保 round-trip 正确。
wiremock 的 up_to_n_times：注意调用顺序——重试测试里第一个 mock 必须有 up_to_n_times(1)，否则它会一直返回 429 永远不到第二个 mock。

## 十七、产出物

完成 spec-04 后你应该有：

完整的 LlmProvider trait 和 GeminiProvider 实现
准确的成本计算和 token 统计
完善的错误处理和重试机制
Mock provider 用于开发和测试
LlmService 业务层支持热重载
2 个新 Tauri command（get_model_info / test_summary_with_images）
完整的单元测试 + 真实 API 集成测试
前后端基础设施完全就位，可以接入截图流水线

下一份 spec：spec-05-database-layer.md——SQLite 初始化、连接池、schema migrations、screenshots / sessions / segments 三张表的 repo 实现。这是最后一份基础设施 spec，spec-06 之后就是真正的业务流水线和 UI 了。
