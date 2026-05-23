一、目标
补全 MemoryProvider trait 的完整方法集，并完成 SupermemoryProvider 的生产级实现。完成本 spec 后：

MemoryProvider trait 包含所有 P0 需要的方法：add / get / list / search / delete / health_check
SupermemoryProvider 完整实现这些方法，能真实读写 Supermemory
提供一个 MockMemoryProvider 用于本地开发和单元测试，不需要真实 API key 也能跑 UI
MemoryService 业务层封装，处理 provider 切换、错误降级、重试
提供完整的 Tauri command 让前端能列出和搜索记忆
完整的 Rust 单元测试和集成测试

关键约束：所有 Supermemory 特有的概念（container_tags、user profile、metadata 字段）必须封装在 SupermemoryProvider 内部，trait 层暴露的是通用 Memory 模型。未来切换到 Mem0 时，业务代码不需要改一行。
二、不做什么

❌ 不实现 SQLite 持久化（spec-05 做）
❌ 不实现截图采集和 Gemini 调用（spec-06/07 做）
❌ 不实现记忆页 UI（spec-09 做，本 spec 只暴露 command）
❌ 不实现 Mem0 / 自建 provider（只搭好 trait 抽象，实现留给未来）
❌ 不做记忆的批量导入导出
❌ 不做记忆的更新（update）—— P0 只读 / 只写 / 只删，不改

三、成功标准
完成本 spec 后：

cargo test 全部通过（包含 mock provider 的单元测试）
配置好真实 Supermemory API key 后，cargo test --test integration_supermemory -- --ignored 能跑通真实 API 集成测试
启动 app 后，前端能调用 list_memories / search_memories 命令拿到数据
当 Supermemory key 未配置时，调用记忆相关命令返回明确的"未配置"错误，而不是 panic
当 Supermemory API 返回 429（限流）时，provider 自动重试 1 次（指数退避）
写入一条 memory，能在 list 里看到、能 search 到、能 delete 掉
序列化测试断言：Memory / MemoryInput 的 JSON 序列化对前端 TS 类型完全对齐

四、Memory 数据模型
4.1 完整的 trait 定义
src-tauri/src/providers/memory/mod.rs：
rustuse async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::error::Result;

pub mod supermemory;
pub mod mock;
pub mod types;

pub use types::*;

#[async_trait]
pub trait MemoryProvider: Send + Sync {
/// 健康检查（已在 spec-02 实现）
async fn health_check(&self) -> Result<()>;

    /// 写入一条记忆，返回 provider 分配的 ID
    async fn add(&self, input: MemoryInput) -> Result<MemoryId>;
    
    /// 根据 ID 获取一条记忆
    async fn get(&self, id: &MemoryId) -> Result<Option<Memory>>;
    
    /// 分页列出记忆，按 occurred_at 倒序
    async fn list(&self, query: ListQuery) -> Result<MemoryPage>;
    
    /// 全文搜索
    async fn search(&self, query: SearchQuery) -> Result<Vec<Memory>>;
    
    /// 删除一条记忆
    async fn delete(&self, id: &MemoryId) -> Result<()>;
}
4.2 通用数据类型
src-tauri/src/providers/memory/types.rs：
rustuse chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type MemoryId = String;

/// 写入记忆时的输入结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryInput {
pub content: String,
pub source: MemorySource,
pub tags: Vec<String>,
pub occurred_at: DateTime<Utc>,
/// 自由形式元数据（provider 内部转换为各自的存储格式）
#[serde(default)]
pub metadata: serde_json::Value,
}

/// 完整的记忆对象
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
pub id: MemoryId,
pub content: String,
pub source: MemorySource,
pub tags: Vec<String>,
pub occurred_at: DateTime<Utc>,
pub created_at: DateTime<Utc>,
#[serde(default)]
pub metadata: serde_json::Value,
}

/// 数据源类型 —— P0 只用 Screenshot 变体
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MemorySource {
Screenshot {
session_id: String,
segment_id: i64,
},
/// P1 阶段扩展：
#[allow(dead_code)]
ClaudeCode { conversation_id: String },
#[allow(dead_code)]
Notion { page_id: String },
#[allow(dead_code)]
Manual,
}

impl MemorySource {
/// 转换为短字符串标签，用于 UI 显示和 provider 内部分类
pub fn kind(&self) -> &'static str {
match self {
MemorySource::Screenshot { .. } => "screenshot",
MemorySource::ClaudeCode { .. } => "claude_code",
MemorySource::Notion { .. } => "notion",
MemorySource::Manual => "manual",
}
}
}

/// 列表查询参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListQuery {
pub limit: usize,
pub offset: usize,
pub date_from: Option<DateTime<Utc>>,
pub date_to: Option<DateTime<Utc>>,
pub source_kind: Option<String>,
}

impl Default for ListQuery {
fn default() -> Self {
Self {
limit: 50,
offset: 0,
date_from: None,
date_to: None,
source_kind: None,
}
}
}

/// 搜索查询参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
pub query: String,
pub limit: usize,
pub source_kind: Option<String>,
}

/// 分页结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPage {
pub items: Vec<Memory>,
pub total: Option<usize>,
pub has_more: bool,
}
关键设计点：

MemorySource 用 enum + serde tag，而不是字符串。这让 P0 只关心 Screenshot 一种来源，但 P1 加新来源时 trait 不需要改、序列化向后兼容。
metadata 字段是 serde_json::Value，给每个 provider 自由发挥的空间。Supermemory 可以在这里塞 container_tags，Mem0 可以塞自己的字段，trait 不感知。
MemoryPage.total 是 Option —— Supermemory 的搜索接口可能不返回总数，做成可选避免强制实现。
source_kind 用字符串而不是 enum，因为 list 查询场景里不需要 enum 的字段（只要类型标签）。

五、SupermemoryProvider 完整实现
5.1 Supermemory API 概览
写代码之前必须先用 curl 跑通基本调用，确认实际的 endpoint、auth 方式、请求/响应格式。这里基于 Supermemory v3 的常见 API 形态写实现，实际编码时以 supermemory.ai 的官方文档为准。
参考的核心 endpoints：

POST /v3/memories 添加记忆
GET /v3/memories/{id} 获取一条
GET /v3/memories?container_tags=...&limit=...&offset=... 列出
POST /v3/search 搜索
DELETE /v3/memories/{id} 删除

Auth: Authorization: Bearer <api_key>
所有 Supermemory 的特定字段如何映射到通用 Memory 字段，需要在 SupermemoryProvider 内部完成转换。这是封装的核心。
5.2 完整实现
src-tauri/src/providers/memory/supermemory.rs：
rustuse super::{
Memory, MemoryId, MemoryInput, MemoryPage, MemoryProvider, MemorySource,
ListQuery, SearchQuery,
};
use crate::error::{CorivoError, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

const SUPERMEMORY_BASE_URL: &str = "https://api.supermemory.ai/v3";
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_RETRIES: u32 = 1;

/// Corivo 写入 Supermemory 时使用的统一 container tag。
/// 让所有 Corivo 写入的记忆能从 Supermemory 里整体筛出来。
const CORIVO_CONTAINER_TAG: &str = "corivo";

pub struct SupermemoryProvider {
api_key: String,
client: Client,
base_url: String,
}

impl SupermemoryProvider {
pub fn new(api_key: String) -> Result<Self> {
let client = Client::builder()
.timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
.build()
.map_err(|e| CorivoError::Network(e.to_string()))?;

        Ok(Self {
            api_key,
            client,
            base_url: SUPERMEMORY_BASE_URL.to_string(),
        })
    }
    
    #[cfg(test)]
    pub fn with_base_url(api_key: String, base_url: String) -> Result<Self> {
        let mut provider = Self::new(api_key)?;
        provider.base_url = base_url;
        Ok(provider)
    }
    
    /// 把通用 MemoryInput 转换成 Supermemory 的请求 payload
    fn to_supermemory_payload(&self, input: &MemoryInput) -> SmCreateRequest {
        // 把 source 编码到 container_tags 里，这样 list 时可以反查
        let mut container_tags = vec![
            CORIVO_CONTAINER_TAG.to_string(),
            format!("source:{}", input.source.kind()),
        ];
        
        // source 内部信息也编码进 metadata
        let mut metadata = match &input.metadata {
            serde_json::Value::Object(m) => m.clone(),
            _ => serde_json::Map::new(),
        };
        metadata.insert(
            "corivo_source".to_string(),
            serde_json::to_value(&input.source).unwrap(),
        );
        metadata.insert(
            "occurred_at".to_string(),
            serde_json::Value::String(input.occurred_at.to_rfc3339()),
        );
        
        // user tags 也加进去
        for tag in &input.tags {
            container_tags.push(format!("user:{}", tag));
        }
        
        SmCreateRequest {
            content: input.content.clone(),
            container_tags,
            metadata: serde_json::Value::Object(metadata),
        }
    }
    
    /// 把 Supermemory 的响应反序列化为通用 Memory
    fn from_supermemory_response(&self, sm: SmMemoryResponse) -> Result<Memory> {
        // 从 metadata 里恢复 source
        let source = sm
            .metadata
            .as_object()
            .and_then(|m| m.get("corivo_source"))
            .and_then(|v| serde_json::from_value::<MemorySource>(v.clone()).ok())
            .unwrap_or(MemorySource::Manual);
        
        let occurred_at = sm
            .metadata
            .as_object()
            .and_then(|m| m.get("occurred_at"))
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or(sm.created_at);
        
        // 从 container_tags 反推 user tags
        let tags: Vec<String> = sm
            .container_tags
            .iter()
            .filter_map(|t| t.strip_prefix("user:").map(String::from))
            .collect();
        
        Ok(Memory {
            id: sm.id,
            content: sm.content,
            source,
            tags,
            occurred_at,
            created_at: sm.created_at,
            metadata: sm.metadata,
        })
    }
    
    /// 通用的请求执行 + 错误映射 + 重试逻辑
    async fn execute<F, Fut, T>(&self, op_name: &str, op: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut attempts = 0u32;
        loop {
            match op().await {
                Ok(v) => return Ok(v),
                Err(CorivoError::Provider(msg)) if msg.contains("rate_limited") => {
                    if attempts >= MAX_RETRIES {
                        return Err(CorivoError::Provider(format!(
                            "{}: 超出重试次数",
                            op_name
                        )));
                    }
                    attempts += 1;
                    let backoff = Duration::from_millis(500 * 2u64.pow(attempts));
                    tracing::warn!("{} rate limited, retry after {:?}", op_name, backoff);
                    sleep(backoff).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
    
    fn map_status_error(&self, status: StatusCode, body: &str) -> CorivoError {
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                CorivoError::Provider("Supermemory: API key 无效".to_string())
            }
            StatusCode::TOO_MANY_REQUESTS => {
                CorivoError::Provider("rate_limited".to_string())
            }
            StatusCode::NOT_FOUND => CorivoError::Provider("not_found".to_string()),
            _ => CorivoError::Provider(format!(
                "Supermemory HTTP {}: {}",
                status, body
            )),
        }
    }
}

#[async_trait]
impl MemoryProvider for SupermemoryProvider {
async fn health_check(&self) -> Result<()> {
// 复用 spec-02 的实现逻辑
let url = format!("{}/search", self.base_url);
let resp = self
.client
.post(&url)
.bearer_auth(&self.api_key)
.json(&serde_json::json!({ "q": "health", "limit": 1 }))
.send()
.await
.map_err(|e| CorivoError::Network(e.to_string()))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(self.map_status_error(status, &body))
        }
    }
    
    async fn add(&self, input: MemoryInput) -> Result<MemoryId> {
        let payload = self.to_supermemory_payload(&input);
        
        self.execute("add_memory", || async {
            let url = format!("{}/memories", self.base_url);
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&payload)
                .send()
                .await
                .map_err(|e| CorivoError::Network(e.to_string()))?;
            
            if resp.status().is_success() {
                let body: SmCreateResponse = resp
                    .json()
                    .await
                    .map_err(|e| CorivoError::Network(e.to_string()))?;
                Ok(body.id)
            } else {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                Err(self.map_status_error(status, &body))
            }
        })
        .await
    }
    
    async fn get(&self, id: &MemoryId) -> Result<Option<Memory>> {
        let url = format!("{}/memories/{}", self.base_url, id);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| CorivoError::Network(e.to_string()))?;
        
        match resp.status() {
            StatusCode::OK => {
                let sm: SmMemoryResponse = resp
                    .json()
                    .await
                    .map_err(|e| CorivoError::Network(e.to_string()))?;
                Ok(Some(self.from_supermemory_response(sm)?))
            }
            StatusCode::NOT_FOUND => Ok(None),
            status => {
                let body = resp.text().await.unwrap_or_default();
                Err(self.map_status_error(status, &body))
            }
        }
    }
    
    async fn list(&self, query: ListQuery) -> Result<MemoryPage> {
        let url = format!("{}/memories", self.base_url);
        let mut req = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .query(&[
                ("container_tags", CORIVO_CONTAINER_TAG),
                ("limit", &query.limit.to_string()),
                ("offset", &query.offset.to_string()),
            ]);
        
        if let Some(kind) = &query.source_kind {
            req = req.query(&[("container_tags", &format!("source:{}", kind))]);
        }
        
        let resp = req
            .send()
            .await
            .map_err(|e| CorivoError::Network(e.to_string()))?;
        
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(self.map_status_error(status, &body));
        }
        
        let body: SmListResponse = resp
            .json()
            .await
            .map_err(|e| CorivoError::Network(e.to_string()))?;
        
        let items: Result<Vec<Memory>> = body
            .items
            .into_iter()
            .map(|sm| self.from_supermemory_response(sm))
            .collect();
        
        Ok(MemoryPage {
            items: items?,
            total: body.total,
            has_more: body.has_more.unwrap_or(false),
        })
    }
    
    async fn search(&self, query: SearchQuery) -> Result<Vec<Memory>> {
        let mut payload = serde_json::json!({
            "q": query.query,
            "limit": query.limit,
            "container_tags": [CORIVO_CONTAINER_TAG],
        });
        
        if let Some(kind) = &query.source_kind {
            payload["container_tags"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::Value::String(format!("source:{}", kind)));
        }
        
        let url = format!("{}/search", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| CorivoError::Network(e.to_string()))?;
        
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(self.map_status_error(status, &body));
        }
        
        let body: SmSearchResponse = resp
            .json()
            .await
            .map_err(|e| CorivoError::Network(e.to_string()))?;
        
        body.results
            .into_iter()
            .map(|sm| self.from_supermemory_response(sm))
            .collect()
    }
    
    async fn delete(&self, id: &MemoryId) -> Result<()> {
        let url = format!("{}/memories/{}", self.base_url, id);
        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| CorivoError::Network(e.to_string()))?;
        
        if resp.status().is_success() || resp.status() == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(self.map_status_error(status, &body))
        }
    }
}

// === Supermemory wire types（仅本模块内部使用）===

#[derive(Debug, Serialize)]
struct SmCreateRequest {
content: String,
container_tags: Vec<String>,
metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct SmCreateResponse {
id: String,
}

#[derive(Debug, Deserialize)]
struct SmMemoryResponse {
id: String,
content: String,
container_tags: Vec<String>,
metadata: serde_json::Value,
created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct SmListResponse {
items: Vec<SmMemoryResponse>,
total: Option<usize>,
has_more: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct SmSearchResponse {
results: Vec<SmMemoryResponse>,
}
关键设计点解释：

CORIVO_CONTAINER_TAG = "corivo"：所有 Corivo 写入的记忆都打上这个 tag。这样如果用户的 Supermemory 账号同时被其他工具用，list 时只会拿到 Corivo 的数据，不会污染。
source 编码到两个地方：container_tags 里加 source:screenshot 用于服务端过滤；metadata.corivo_source 里存完整的 source enum 用于反序列化。这是 Supermemory 的字符串 tag 限制下能做到的最优解。
execute 方法：所有需要重试的操作都走这个壳，集中处理 429 重试。get 和 delete 这种幂等读不需要重试（也可以加，看你想多保险）。
with_base_url 是 #[cfg(test)] ：让测试可以指向 mock server（比如 wiremock 或 httpmock），不污染生产代码。

六、MockMemoryProvider
6.1 用途

没配 Supermemory key 时让 UI 也能跑起来看效果
单元测试时不需要网络
演示场景预填假数据

6.2 实现
src-tauri/src/providers/memory/mock.rs：
rustuse super::{
ListQuery, Memory, MemoryId, MemoryInput, MemoryPage, MemoryProvider, MemorySource,
SearchQuery,
};
use crate::error::Result;
use async_trait::async_trait;
use chrono::Utc;
use std::sync::RwLock;
use uuid::Uuid;

pub struct MockMemoryProvider {
storage: RwLock<Vec<Memory>>,
}

impl MockMemoryProvider {
pub fn new() -> Self {
Self {
storage: RwLock::new(Vec::new()),
}
}

    pub fn with_demo_data() -> Self {
        let now = Utc::now();
        let demo = vec![
            Memory {
                id: "demo-1".to_string(),
                content: "用户在 VS Code 里调试 auth 模块的 JWT token 刷新逻辑，反复查看 RefreshTokenService。".to_string(),
                source: MemorySource::Screenshot {
                    session_id: "demo-session".to_string(),
                    segment_id: 1,
                },
                tags: vec!["coding".to_string()],
                occurred_at: now - chrono::Duration::minutes(45),
                created_at: now - chrono::Duration::minutes(45),
                metadata: serde_json::json!({}),
            },
            Memory {
                id: "demo-2".to_string(),
                content: "用户在飞书群里讨论新需求，整理出 3 条待确认问题，主要关于支付方式和退款流程。".to_string(),
                source: MemorySource::Screenshot {
                    session_id: "demo-session".to_string(),
                    segment_id: 2,
                },
                tags: vec!["communication".to_string()],
                occurred_at: now - chrono::Duration::minutes(30),
                created_at: now - chrono::Duration::minutes(30),
                metadata: serde_json::json!({}),
            },
        ];
        Self {
            storage: RwLock::new(demo),
        }
    }
}

#[async_trait]
impl MemoryProvider for MockMemoryProvider {
async fn health_check(&self) -> Result<()> {
Ok(())
}

    async fn add(&self, input: MemoryInput) -> Result<MemoryId> {
        let id = Uuid::new_v4().to_string();
        let memory = Memory {
            id: id.clone(),
            content: input.content,
            source: input.source,
            tags: input.tags,
            occurred_at: input.occurred_at,
            created_at: Utc::now(),
            metadata: input.metadata,
        };
        self.storage.write().unwrap().push(memory);
        Ok(id)
    }
    
    async fn get(&self, id: &MemoryId) -> Result<Option<Memory>> {
        Ok(self
            .storage
            .read()
            .unwrap()
            .iter()
            .find(|m| &m.id == id)
            .cloned())
    }
    
    async fn list(&self, query: ListQuery) -> Result<MemoryPage> {
        let storage = self.storage.read().unwrap();
        let mut items: Vec<Memory> = storage
            .iter()
            .filter(|m| {
                if let Some(from) = query.date_from {
                    if m.occurred_at < from {
                        return false;
                    }
                }
                if let Some(to) = query.date_to {
                    if m.occurred_at > to {
                        return false;
                    }
                }
                if let Some(kind) = &query.source_kind {
                    if m.source.kind() != kind {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();
        
        items.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
        
        let total = items.len();
        let paged: Vec<Memory> = items
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect();
        let has_more = query.offset + paged.len() < total;
        
        Ok(MemoryPage {
            items: paged,
            total: Some(total),
            has_more,
        })
    }
    
    async fn search(&self, query: SearchQuery) -> Result<Vec<Memory>> {
        let q = query.query.to_lowercase();
        let storage = self.storage.read().unwrap();
        let mut results: Vec<Memory> = storage
            .iter()
            .filter(|m| m.content.to_lowercase().contains(&q))
            .filter(|m| {
                if let Some(kind) = &query.source_kind {
                    m.source.kind() == kind
                } else {
                    true
                }
            })
            .cloned()
            .collect();
        results.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
        results.truncate(query.limit);
        Ok(results)
    }
    
    async fn delete(&self, id: &MemoryId) -> Result<()> {
        let mut storage = self.storage.write().unwrap();
        storage.retain(|m| &m.id != id);
        Ok(())
    }
}
依赖新增：
tomluuid = { version = "1", features = ["v4"] }
七、MemoryService 业务层
src-tauri/src/services/memory_service.rs：
rustuse crate::error::{CorivoError, Result};
use crate::providers::memory::{
mock::MockMemoryProvider,
supermemory::SupermemoryProvider,
ListQuery, Memory, MemoryId, MemoryInput, MemoryPage, MemoryProvider, SearchQuery,
};
use crate::services::keychain_service::{KeychainKey, KeychainService};
use std::sync::{Arc, RwLock};

/// MemoryService 是业务层包装：
/// - 持有当前 active 的 provider
/// - 当用户更新 API key 时能热重载 provider
/// - 当 key 缺失时自动降级到 MockMemoryProvider（开发友好）
pub struct MemoryService {
provider: RwLock<Arc<dyn MemoryProvider>>,
keychain: Arc<KeychainService>,
use_mock_when_unconfigured: bool,
}

impl MemoryService {
pub fn new(keychain: Arc<KeychainService>) -> Self {
let provider = Self::build_provider(&keychain, true);
Self {
provider: RwLock::new(provider),
keychain,
use_mock_when_unconfigured: true,
}
}

    fn build_provider(
        keychain: &KeychainService,
        use_mock_when_unconfigured: bool,
    ) -> Arc<dyn MemoryProvider> {
        match keychain.load(KeychainKey::SupermemoryApiKey) {
            Ok(Some(key)) => match SupermemoryProvider::new(key) {
                Ok(p) => Arc::new(p),
                Err(e) => {
                    tracing::error!("failed to init SupermemoryProvider: {:?}, fallback to mock", e);
                    Arc::new(MockMemoryProvider::with_demo_data())
                }
            },
            _ => {
                if use_mock_when_unconfigured {
                    tracing::warn!("Supermemory API key not configured, using MockMemoryProvider");
                    Arc::new(MockMemoryProvider::with_demo_data())
                } else {
                    Arc::new(MockMemoryProvider::new())
                }
            }
        }
    }
    
    /// 当配置变化时调用，重新构建 provider
    pub fn reload(&self) {
        let new_provider = Self::build_provider(&self.keychain, self.use_mock_when_unconfigured);
        *self.provider.write().unwrap() = new_provider;
        tracing::info!("MemoryService provider reloaded");
    }
    
    fn current(&self) -> Arc<dyn MemoryProvider> {
        self.provider.read().unwrap().clone()
    }
    
    pub async fn add(&self, input: MemoryInput) -> Result<MemoryId> {
        self.current().add(input).await
    }
    
    pub async fn get(&self, id: &MemoryId) -> Result<Option<Memory>> {
        self.current().get(id).await
    }
    
    pub async fn list(&self, query: ListQuery) -> Result<MemoryPage> {
        self.current().list(query).await
    }
    
    pub async fn search(&self, query: SearchQuery) -> Result<Vec<Memory>> {
        self.current().search(query).await
    }
    
    pub async fn delete(&self, id: &MemoryId) -> Result<()> {
        self.current().delete(id).await
    }
    
    pub async fn health_check(&self) -> Result<()> {
        self.current().health_check().await
    }
}
关键设计点：

RwLock<Arc<dyn MemoryProvider>>：让 provider 能在运行时被替换（用户更新 key 后），同时读取是无锁的（.clone() Arc 很便宜）。
reload()：spec-02 的 save_supermemory_api_key 命令在保存成功后必须调用 MemoryService::reload()，否则旧 provider 还在用旧 key。
自动降级到 Mock：开发阶段没配 key 也能跑 UI，减少摩擦。生产打包时可以通过环境变量关掉这个降级。

八、Tauri Commands
src-tauri/src/commands/memory.rs（新增）：
rustuse crate::providers::memory::{ListQuery, Memory, MemoryPage, SearchQuery};
use crate::services::memory_service::MemoryService;
use std::sync::Arc;
use tauri::State;

pub struct MemoryAppState {
pub memory_service: Arc<MemoryService>,
}

#[tauri::command]
pub async fn list_memories(
state: State<'_, MemoryAppState>,
limit: usize,
offset: usize,
source_kind: Option<String>,
) -> Result<MemoryPage, String> {
let query = ListQuery {
limit,
offset,
date_from: None,
date_to: None,
source_kind,
};
state.memory_service.list(query).await.map_err(Into::into)
}

#[tauri::command]
pub async fn search_memories(
state: State<'_, MemoryAppState>,
query: String,
limit: usize,
source_kind: Option<String>,
) -> Result<Vec<Memory>, String> {
let q = SearchQuery { query, limit, source_kind };
state.memory_service.search(q).await.map_err(Into::into)
}

#[tauri::command]
pub async fn get_memory(
state: State<'_, MemoryAppState>,
id: String,
) -> Result<Option<Memory>, String> {
state.memory_service.get(&id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn delete_memory(
state: State<'_, MemoryAppState>,
id: String,
) -> Result<(), String> {
state.memory_service.delete(&id).await.map_err(Into::into)
}
更新 lib.rs 注册 commands 和 state：
rust// 在 setup 里增加
let memory_service = Arc::new(MemoryService::new(keychain_service.clone()));
app.manage(MemoryAppState { memory_service: memory_service.clone() });

// invoke_handler 里增加
commands::memory::list_memories,
commands::memory::search_memories,
commands::memory::get_memory,
commands::memory::delete_memory,
并且修改 commands/config.rs 里的 save_supermemory_api_key，保存成功后调用 memory_service.reload()：
rust#[tauri::command]
pub async fn save_supermemory_api_key(
state: State<'_, AppState>,
memory_state: State<'_, MemoryAppState>,
api_key: String,
) -> Result<(), String> {
state
.keychain_service
.save(KeychainKey::SupermemoryApiKey, &api_key)
.map_err(String::from)?;
memory_state.memory_service.reload();
Ok(())
}
九、前端类型和封装
src/lib/types.ts 增加：
tsexport type MemorySourceKind = 'screenshot' | 'claude_code' | 'notion' | 'manual'

export type MemorySource =
| { type: 'screenshot'; session_id: string; segment_id: number }
| { type: 'claude_code'; conversation_id: string }
| { type: 'notion'; page_id: string }
| { type: 'manual' }

export interface Memory {
id: string
content: string
source: MemorySource
tags: string[]
occurred_at: string
created_at: string
metadata: Record<string, unknown>
}

export interface MemoryPage {
items: Memory[]
total: number | null
has_more: boolean
}
src/lib/tauri.ts 增加：
tsimport type { Memory, MemoryPage } from './types'

export async function listMemories(
limit: number,
offset: number,
sourceKind?: string
): Promise<MemoryPage> {
return invoke<MemoryPage>('list_memories', { limit, offset, sourceKind: sourceKind ?? null })
}

export async function searchMemories(
query: string,
limit: number,
sourceKind?: string
): Promise<Memory[]> {
return invoke<Memory[]>('search_memories', { query, limit, sourceKind: sourceKind ?? null })
}

export async function getMemory(id: string): Promise<Memory | null> {
return invoke<Memory | null>('get_memory', { id })
}

export async function deleteMemory(id: string): Promise<void> {
return invoke<void>('delete_memory', { id })
}
注意：本 spec 不实现 记忆页 UI，那是 spec-09 的事。本 spec 只暴露 command。
十、测试
10.1 单元测试
src-tauri/src/providers/memory/mock.rs 末尾追加：
rust#[cfg(test)]
mod tests {
use super::*;
use chrono::Duration;

    fn make_input(content: &str) -> MemoryInput {
        MemoryInput {
            content: content.to_string(),
            source: MemorySource::Screenshot {
                session_id: "s1".to_string(),
                segment_id: 1,
            },
            tags: vec![],
            occurred_at: Utc::now(),
            metadata: serde_json::json!({}),
        }
    }
    
    #[tokio::test]
    async fn test_add_get_delete() {
        let p = MockMemoryProvider::new();
        let id = p.add(make_input("hello")).await.unwrap();
        let m = p.get(&id).await.unwrap();
        assert!(m.is_some());
        assert_eq!(m.unwrap().content, "hello");
        p.delete(&id).await.unwrap();
        assert!(p.get(&id).await.unwrap().is_none());
    }
    
    #[tokio::test]
    async fn test_list_pagination() {
        let p = MockMemoryProvider::new();
        for i in 0..5 {
            p.add(make_input(&format!("memory {}", i))).await.unwrap();
        }
        let page = p.list(ListQuery { limit: 2, offset: 0, ..Default::default() }).await.unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.has_more);
        assert_eq!(page.total, Some(5));
    }
    
    #[tokio::test]
    async fn test_search() {
        let p = MockMemoryProvider::new();
        p.add(make_input("auth module refactor")).await.unwrap();
        p.add(make_input("payment integration")).await.unwrap();
        let results = p.search(SearchQuery {
            query: "auth".to_string(),
            limit: 10,
            source_kind: None,
        }).await.unwrap();
        assert_eq!(results.len(), 1);
    }
}
10.2 序列化对齐测试
src-tauri/src/providers/memory/types.rs 末尾追加：
rust#[cfg(test)]
mod tests {
use super::*;

    #[test]
    fn test_memory_source_serialization() {
        let s = MemorySource::Screenshot {
            session_id: "s1".to_string(),
            segment_id: 42,
        };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["type"], "screenshot");
        assert_eq!(json["session_id"], "s1");
        assert_eq!(json["segment_id"], 42);
    }
    
    #[test]
    fn test_memory_source_roundtrip() {
        let original = MemorySource::Screenshot {
            session_id: "s1".to_string(),
            segment_id: 42,
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: MemorySource = serde_json::from_str(&json).unwrap();
        match parsed {
            MemorySource::Screenshot { session_id, segment_id } => {
                assert_eq!(session_id, "s1");
                assert_eq!(segment_id, 42);
            }
            _ => panic!("wrong variant"),
        }
    }
}
10.3 Supermemory 集成测试（手动跑）
src-tauri/tests/integration_supermemory.rs：
rust//! 真实 Supermemory API 集成测试
//! 跑法: SUPERMEMORY_API_KEY=xxx cargo test --test integration_supermemory -- --ignored

use corivo::providers::memory::{supermemory::SupermemoryProvider, *};
use chrono::Utc;

fn get_key() -> String {
std::env::var("SUPERMEMORY_API_KEY").expect("SUPERMEMORY_API_KEY not set")
}

#[tokio::test]
#[ignore]
async fn test_full_lifecycle() {
let provider = SupermemoryProvider::new(get_key()).unwrap();

    // Add
    let input = MemoryInput {
        content: format!("Corivo integration test {}", Utc::now().timestamp()),
        source: MemorySource::Screenshot {
            session_id: "test-session".to_string(),
            segment_id: 1,
        },
        tags: vec!["integration_test".to_string()],
        occurred_at: Utc::now(),
        metadata: serde_json::json!({}),
    };
    let id = provider.add(input).await.expect("add failed");
    println!("created memory: {}", id);
    
    // Get
    let memory = provider.get(&id).await.expect("get failed");
    assert!(memory.is_some());
    
    // Search
    let results = provider.search(SearchQuery {
        query: "integration test".to_string(),
        limit: 10,
        source_kind: Some("screenshot".to_string()),
    }).await.expect("search failed");
    assert!(!results.is_empty());
    
    // Delete
    provider.delete(&id).await.expect("delete failed");
    let after = provider.get(&id).await.expect("get after delete failed");
    assert!(after.is_none());
}
十一、任务分解
会话范围完成判定1types.rs + trait 完整定义 + 序列化测试cargo test types::tests 全过2MockMemoryProvider 完整实现 + 单元测试cargo test mock::tests 全过3SupermemoryProvider 完整实现 + 错误映射 + 重试cargo build 通过4MemoryService 业务层 + reload 机制编译通过5commands/memory.rs + lib.rs 装配 + save key reload 钩子启动 app 后能 invoke list_memories 拿到 mock demo data6前端 types + tauri.ts 封装类型对齐，无 TS 报错7Supermemory 集成测试（手动 + 真实 key）手动跑通完整 lifecycle
十二、验收清单

cargo test 全部通过
cargo clippy 无 warning
无 Supermemory key 时启动 app，调用 list_memories 返回 demo 数据
配置正确 Supermemory key 后，再次调用 list_memories 返回真实数据（验证 reload 生效）
配置错误 Supermemory key 后，调用 list_memories 返回明确错误信息
写入一条 memory → list 能看到 → search 能找到 → delete 能删掉
cargo test --test integration_supermemory -- --ignored 在配好真实 key 时通过
MemorySource 的 JSON 序列化字段和前端 TS 类型完全对齐
前端 import 'listMemories' / 'searchMemories' 能拿到正确类型提示

十三、坑点预警

Supermemory API 实际字段以官方为准：本 spec 写的 wire types（SmCreateRequest 等）是基于推测的常见形态。写代码前必须先 curl 一遍真实 API，校对字段名、auth 方式、返回结构。Supermemory 文档迭代快，可能和 spec 写的不一致，以 curl 结果为准，spec 不必修改。
container_tags 的语义：Supermemory 的 container_tags 是用来做用户/容器隔离的，不是普通 tags。一个用户的所有 Corivo 数据共享一个 container（即 corivo），不要每条数据用不同 container。
reload() 的并发安全：用户在写入过程中改 key 触发 reload，正在跑的 add 请求会用旧 provider 完成（因为 current() 已经 clone 了 Arc），新请求会用新 provider。这是正确行为，不要试图取消正在跑的请求。
Mock 和真实 provider 的行为差异：Mock 的 search 是子串匹配，Supermemory 的 search 是语义检索。同一个 query 在两边可能返回不同的结果集。测试时不要假设 mock 和真实 provider 行为完全一致，只断言"某个内容能被搜到"这种基本性质。
uuid crate 的 features：必须开 v4。
async_trait 的开销：每次方法调用有一次堆分配，但对于网络 IO 主导的场景完全可以忽略。
删除是否真删：Supermemory 的 delete 可能是软删除。如果 delete 后立刻 list 还能看到这条记忆，是正常的。不要在删除后立刻断言列表里没有它——延迟一秒再查或者用 get 验证（get 会立即返回 404）。
save_supermemory_api_key 现在依赖两个 State：注意 Tauri command 的多 State 注入语法。

十四、产出物
完成 spec-03 后你应该有：

完整的 MemoryProvider 抽象层和 Supermemory 实现
一个开发友好的 Mock provider
MemoryService 业务层支持热重载
4 个新的 Tauri command 暴露给前端
完整的单元测试和可手动跑的集成测试
前端能拿到类型化的 Memory 列表和搜索结果（虽然 UI 还没做）

