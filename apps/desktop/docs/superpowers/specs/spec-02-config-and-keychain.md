# spec-02-config-and-keychain.md

## 一、目标

实现 Corivo 的配置存储和密钥管理基础设施。完成本 spec 后：
- 后端有一套完整的 Config 数据结构和读写服务
- 普通配置存到 tauri-plugin-store（明文 JSON）
- 敏感 API key 存到系统 keychain（macOS Keychain / Windows Credential Manager / Linux Secret Service）
- 前端有一个能用的「配置 → API Keys」子页，用户能填 Gemini 和 Supermemory 的 key 并点「测试连接」
- 测试连接调用真实的 Gemini 和 Supermemory API 做 health check
- 所有配置项持久化，重启应用后能恢复

## 二、不做什么

- ❌ 不实现完整的 Settings 页面（只做 API Keys 子页，其他子页 spec-11 完成）
- ❌ 不实现 MemoryProvider / LlmProvider 的完整业务方法（只做 health_check）
- ❌ 不实现配置项的高级校验（比如 Gemini key 格式校验）
- ❌ 不做配置的导入导出
- ❌ 不做配置变更的实时通知（事件总线 spec-06 引入）

## 三、成功标准

  完成本 spec 后：
1. 启动 app，进入「配置」页，能看到「API Keys」子项被默认选中
2. 看到两个 input：Gemini API Key、Supermemory API Key，每个旁边有「测试」按钮
3. 不填写任何东西时，input 是空的；填写后点保存，重启 app 后仍然能从输入框看到 key（注意：实际是从 keychain 读出来的，前端只在编辑时短暂持有明文）
4. 点击「测试」按钮：
- Gemini key 正确：toast 显示「Gemini 连接成功」
- Gemini key 错误：toast 显示「Gemini 连接失败：xxx」
- Supermemory 同上
5. 测试期间按钮显示 loading 状态
6. macOS 系统 Keychain Access 应用里能看到 com.corivo.app 的两条记录
7. config.json 里绝对不出现 API key 明文
8. Rust 单元测试覆盖 ConfigService 和 KeychainService 的核心方法

## 四、技术决策
暂时无法在飞书文档外展示此内容

## 五、Rust 后端实现

### 5.1 新增依赖

   src-tauri/Cargo.toml：
   toml
   [dependencies]
# ... 已有依赖
tauri-plugin-store = "2"
keyring = "3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
anyhow = "1"
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
注意 reqwest 用 rustls-tls 而不是 default 的 native-tls，避免 macOS / Linux 的 OpenSSL 依赖问题。

### 5.2 目录结构

src-tauri/src/
├── main.rs
├── lib.rs
├── commands/
│   ├── mod.rs
│   ├── demo.rs                 spec-01 的 greet（保留）
│   └── config.rs               本 spec 新增
├── services/
│   ├── mod.rs
│   ├── config_service.rs       本 spec 新增
│   └── keychain_service.rs     本 spec 新增
├── providers/
│   ├── mod.rs
│   ├── memory/
│   │   ├── mod.rs
│   │   └── supermemory.rs      本 spec 仅实现 health_check
│   └── llm/
│       ├── mod.rs
│       └── gemini.rs           本 spec 仅实现 health_check
├── domain/
│   ├── mod.rs
│   └── config.rs               本 spec 新增：Config struct
└── error.rs                    本 spec 新增：统一 error 类型

### 5.3 Config 数据结构

src-tauri/src/domain/config.rs：
rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
pub capture: CaptureConfig,
pub summary: SummaryConfig,
pub app: AppConfig,
pub memory: MemoryConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
pub interval_secs: u64,
pub segment_duration_mins: u64,
pub max_storage_gb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryConfig {
pub prompt_template: String,
pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
pub auto_start: bool,
pub minimize_to_tray: bool,
pub notifications_enabled: bool,
pub theme: ThemeOption,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeOption {
Light,
Dark,
System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
pub provider: MemoryProviderKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryProviderKind {
Supermemory,
}

impl Default for Config {
fn default() -> Self {
Self {
capture: CaptureConfig {
interval_secs: 30,
segment_duration_mins: 15,
max_storage_gb: 5,
},
summary: SummaryConfig {
prompt_template: DEFAULT_PROMPT.to_string(),
model: "gemini-2.0-flash".to_string(),
},
app: AppConfig {
auto_start: false,
minimize_to_tray: true,
notifications_enabled: true,
theme: ThemeOption::System,
},
memory: MemoryConfig {
provider: MemoryProviderKind::Supermemory,
},
}
}
}

const DEFAULT_PROMPT: &str = "以下是用户在过去 15 分钟内的屏幕截图（按时间顺序），每 30 秒一张。请用 2-3 句话总结用户在做什么，重点描述具体的任务和上下文。如果用户切换了任务，明确指出切换点。";
关键约束：Config 里绝对不包含 API key 字段。API key 单独走 keychain 通道。

### 5.4 错误类型

src-tauri/src/error.rs：
rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CorivoError {
#[error("config error: {0}")]
Config(String),

    #[error("keychain error: {0}")]
    Keychain(String),
    
    #[error("provider error: {0}")]
    Provider(String),
    
    #[error("network error: {0}")]
    Network(String),
    
    #[error("internal: {0}")]
    Internal(String),
}

impl CorivoError {
/// 转换成对前端友好的字符串
pub fn user_message(&self) -> String {
self.to_string()
}
}

// Tauri command 返回时把所有错误转 String
impl From<CorivoError> for String {
fn from(e: CorivoError) -> Self {
e.user_message()
}
}

// 让 anyhow / keyring / reqwest 错误能自动转换
impl From<keyring::Error> for CorivoError {
fn from(e: keyring::Error) -> Self {
CorivoError::Keychain(e.to_string())
}
}

impl From<reqwest::Error> for CorivoError {
fn from(e: reqwest::Error) -> Self {
CorivoError::Network(e.to_string())
}
}

impl From<serde_json::Error> for CorivoError {
fn from(e: serde_json::Error) -> Self {
CorivoError::Internal(format!("json: {}", e))
}
}

pub type Result<T> = std::result::Result<T, CorivoError>;

### 5.5 KeychainService

src-tauri/src/services/keychain_service.rs：
rust
use crate::error::{CorivoError, Result};
use keyring::Entry;

const SERVICE_NAME: &str = "com.corivo.app";

#[derive(Debug, Clone, Copy)]
pub enum KeychainKey {
GeminiApiKey,
SupermemoryApiKey,
}

impl KeychainKey {
fn account_name(&self) -> &'static str {
match self {
KeychainKey::GeminiApiKey => "gemini_api_key",
KeychainKey::SupermemoryApiKey => "supermemory_api_key",
}
}
}

pub struct KeychainService;

impl KeychainService {
pub fn new() -> Self {
Self
}

    pub fn save(&self, key: KeychainKey, value: &str) -> Result<()> {
        let entry = Entry::new(SERVICE_NAME, key.account_name())?;
        entry.set_password(value)?;
        Ok(())
    }
    
    pub fn load(&self, key: KeychainKey) -> Result<Option<String>> {
        let entry = Entry::new(SERVICE_NAME, key.account_name())?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CorivoError::Keychain(e.to_string())),
        }
    }
    
    pub fn delete(&self, key: KeychainKey) -> Result<()> {
        let entry = Entry::new(SERVICE_NAME, key.account_name())?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CorivoError::Keychain(e.to_string())),
        }
    }
    
    /// 检查某个 key 是否已配置（不返回值，只返回布尔）
    pub fn exists(&self, key: KeychainKey) -> Result<bool> {
        Ok(self.load(key)?.is_some())
    }
}

#[cfg(test)]
mod tests {
use super::*;

    // 注意：keychain 测试在 CI 环境可能失败（需要 unlock keychain）
    // 用 #[ignore] 标记，本地手动跑：cargo test -- --ignored
    #[test]
    #[ignore]
    fn test_save_load_delete() {
        let svc = KeychainService::new();
        let key = KeychainKey::GeminiApiKey;
        
        // 清理可能的残留
        let _ = svc.delete(key);
        
        assert!(!svc.exists(key).unwrap());
        
        svc.save(key, "test_value_123").unwrap();
        assert!(svc.exists(key).unwrap());
        assert_eq!(svc.load(key).unwrap(), Some("test_value_123".to_string()));
        
        svc.delete(key).unwrap();
        assert!(!svc.exists(key).unwrap());
    }
}

### 5.6 ConfigService

src-tauri/src/services/config_service.rs：
rust
use crate::domain::config::Config;
use crate::error::{CorivoError, Result};
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_store::{Store, StoreExt};

const STORE_FILENAME: &str = "config.json";
const CONFIG_KEY: &str = "config";

pub struct ConfigService {
store: Arc<Store<Wry>>,
cache: RwLock<Config>,
}

impl ConfigService {
pub fn new(app: &AppHandle) -> Result<Self> {
let store = app
.store(STORE_FILENAME)
.map_err(|e| CorivoError::Config(e.to_string()))?;

        let config = Self::load_from_store(&store)?;
        
        Ok(Self {
            store,
            cache: RwLock::new(config),
        })
    }
    
    fn load_from_store(store: &Store<Wry>) -> Result<Config> {
        match store.get(CONFIG_KEY) {
            Some(value) => {
                let config: Config = serde_json::from_value(value)?;
                Ok(config)
            }
            None => {
                let default = Config::default();
                let value = serde_json::to_value(&default)?;
                store.set(CONFIG_KEY, value);
                store
                    .save()
                    .map_err(|e| CorivoError::Config(e.to_string()))?;
                Ok(default)
            }
        }
    }
    
    pub fn get(&self) -> Config {
        self.cache.read().unwrap().clone()
    }
    
    pub fn update(&self, new_config: Config) -> Result<()> {
        let value = serde_json::to_value(&new_config)?;
        self.store.set(CONFIG_KEY, value);
        self.store
            .save()
            .map_err(|e| CorivoError::Config(e.to_string()))?;
        *self.cache.write().unwrap() = new_config;
        Ok(())
    }
}

### 5.7 MemoryProvider trait（最小版）

src-tauri/src/providers/memory/mod.rs：
rust
use async_trait::async_trait;
use crate::error::Result;

#[async_trait]
pub trait MemoryProvider: Send + Sync {
async fn health_check(&self) -> Result<()>;

    // P0 后续 spec 会陆续添加：
    // async fn add(...) -> Result<MemoryId>;
    // async fn search(...) -> Result<Vec<Memory>>;
    // async fn list(...) -> Result<MemoryPage>;
}

pub mod supermemory;
src-tauri/src/providers/memory/supermemory.rs：
rust
use super::MemoryProvider;
use crate::error::{CorivoError, Result};
use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;

const SUPERMEMORY_BASE_URL: &str = "https://api.supermemory.ai/v3";

pub struct SupermemoryProvider {
api_key: String,
client: Client,
}

impl SupermemoryProvider {
pub fn new(api_key: String) -> Result<Self> {
let client = Client::builder()
.timeout(Duration::from_secs(15))
.build()?;
Ok(Self { api_key, client })
}
}

#[async_trait]
impl MemoryProvider for SupermemoryProvider {
async fn health_check(&self) -> Result<()> {
// Supermemory 没有专用 health endpoint，
// 用一个最小的搜索请求来验证 key 有效性
let url = format!("{}/search", SUPERMEMORY_BASE_URL);
let resp = self
.client
.post(&url)
.bearer_auth(&self.api_key)
.json(&serde_json::json!({
"q": "health_check",
"limit": 1
}))
.send()
.await?;

        if resp.status().is_success() {
            Ok(())
        } else if resp.status() == 401 || resp.status() == 403 {
            Err(CorivoError::Provider("Supermemory: 认证失败，请检查 API key".to_string()))
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(CorivoError::Provider(format!(
                "Supermemory: HTTP {} - {}",
                status, body
            )))
        }
    }
}
注意：Supermemory 的实际 endpoint 路径以官方文档为准，写代码前先 curl 一下他们的 API 确认。如果他们有专用的 /health 或 /me endpoint，优先用那个，更轻量。

### 5.8 LlmProvider trait（最小版）

src-tauri/src/providers/llm/mod.rs：
rust
use async_trait::async_trait;
use crate::error::Result;

#[async_trait]
pub trait LlmProvider: Send + Sync {
async fn health_check(&self) -> Result<()>;
}

pub mod gemini;
src-tauri/src/providers/llm/gemini.rs：
rust
use super::LlmProvider;
use crate::error::{CorivoError, Result};
use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;

const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

pub struct GeminiProvider {
api_key: String,
client: Client,
}

impl GeminiProvider {
pub fn new(api_key: String) -> Result<Self> {
let client = Client::builder()
.timeout(Duration::from_secs(15))
.build()?;
Ok(Self { api_key, client })
}
}

#[async_trait]
impl LlmProvider for GeminiProvider {
async fn health_check(&self) -> Result<()> {
// 用 list models endpoint 做 health check，最轻量
let url = format!("{}/models?key={}", GEMINI_BASE_URL, self.api_key);
let resp = self.client.get(&url).send().await?;

        if resp.status().is_success() {
            Ok(())
        } else if resp.status() == 400 || resp.status() == 401 || resp.status() == 403 {
            Err(CorivoError::Provider(
                "Gemini: API key 无效，请检查".to_string(),
            ))
        } else {
            let status = resp.status();
            Err(CorivoError::Provider(format!("Gemini: HTTP {}", status)))
        }
    }
}

### 5.9 Tauri commands

src-tauri/src/commands/config.rs：
rust
use crate::domain::config::Config;
use crate::providers::{
llm::{gemini::GeminiProvider, LlmProvider},
memory::{supermemory::SupermemoryProvider, MemoryProvider},
};
use crate::services::{
config_service::ConfigService,
keychain_service::{KeychainKey, KeychainService},
};
use std::sync::Arc;
use tauri::State;

pub struct AppState {
pub config_service: Arc<ConfigService>,
pub keychain_service: Arc<KeychainService>,
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<Config, String> {
Ok(state.config_service.get())
}

#[tauri::command]
pub async fn set_config(
state: State<'_, AppState>,
config: Config,
) -> Result<(), String> {
state.config_service.update(config).map_err(Into::into)
}

#[tauri::command]
pub async fn get_api_key_status(
state: State<'_, AppState>,
) -> Result<ApiKeyStatus, String> {
let gemini = state
.keychain_service
.exists(KeychainKey::GeminiApiKey)
.map_err(String::from)?;
let supermemory = state
.keychain_service
.exists(KeychainKey::SupermemoryApiKey)
.map_err(String::from)?;
Ok(ApiKeyStatus {
gemini_configured: gemini,
supermemory_configured: supermemory,
})
}

#[tauri::command]
pub async fn save_gemini_api_key(
state: State<'_, AppState>,
api_key: String,
) -> Result<(), String> {
state
.keychain_service
.save(KeychainKey::GeminiApiKey, &api_key)
.map_err(String::from)
}

#[tauri::command]
pub async fn save_supermemory_api_key(
state: State<'_, AppState>,
api_key: String,
) -> Result<(), String> {
state
.keychain_service
.save(KeychainKey::SupermemoryApiKey, &api_key)
.map_err(String::from)
}

#[tauri::command]
pub async fn delete_gemini_api_key(
state: State<'_, AppState>,
) -> Result<(), String> {
state
.keychain_service
.delete(KeychainKey::GeminiApiKey)
.map_err(String::from)
}

#[tauri::command]
pub async fn delete_supermemory_api_key(
state: State<'_, AppState>,
) -> Result<(), String> {
state
.keychain_service
.delete(KeychainKey::SupermemoryApiKey)
.map_err(String::from)
}

#[tauri::command]
pub async fn test_gemini_connection(api_key: String) -> Result<(), String> {
let provider = GeminiProvider::new(api_key).map_err(String::from)?;
provider.health_check().await.map_err(String::from)
}

#[tauri::command]
pub async fn test_supermemory_connection(api_key: String) -> Result<(), String> {
let provider = SupermemoryProvider::new(api_key).map_err(String::from)?;
provider.health_check().await.map_err(String::from)
}

#[derive(Debug, serde::Serialize)]
pub struct ApiKeyStatus {
pub gemini_configured: bool,
pub supermemory_configured: bool,
}
关键设计：test_* command 不依赖 AppState 里已保存的 key，而是接收前端传过来的 key。这样用户可以在还没保存的时候先测试，更符合直觉。

### 5.10 main.rs / lib.rs 装配

src-tauri/src/lib.rs：
rust
mod commands;
mod domain;
mod error;
mod providers;
mod services;

use commands::config::AppState;
use services::{config_service::ConfigService, keychain_service::KeychainService};
use std::sync::Arc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
tauri::Builder::default()
.plugin(tauri_plugin_shell::init())
.plugin(tauri_plugin_store::Builder::new().build())
.setup(|app| {
let config_service = Arc::new(
ConfigService::new(&app.handle())
.expect("failed to init ConfigService"),
);
let keychain_service = Arc::new(KeychainService::new());

            app.manage(AppState {
                config_service,
                keychain_service,
            });
            
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::demo::greet,
            commands::config::get_config,
            commands::config::set_config,
            commands::config::get_api_key_status,
            commands::config::save_gemini_api_key,
            commands::config::save_supermemory_api_key,
            commands::config::delete_gemini_api_key,
            commands::config::delete_supermemory_api_key,
            commands::config::test_gemini_connection,
            commands::config::test_supermemory_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

### 5.11 capabilities 配置

src-tauri/capabilities/default.json 增加 store plugin 权限：
json
{
"$schema": "../gen/schemas/desktop-schema.json",
"identifier": "default",
"description": "default capabilities",
"windows": ["main"],
"permissions": [
"core:default",
"shell:allow-open",
"store:default"
]
}

## 六、前端实现

### 6.1 类型定义

src/lib/types.ts（在 spec-01 的基础上扩充）：
ts
export type ThemeOption = 'light' | 'dark' | 'system'
export type MemoryProviderKind = 'supermemory'

export interface CaptureConfig {
interval_secs: number
segment_duration_mins: number
max_storage_gb: number
}

export interface SummaryConfig {
prompt_template: string
model: string
}

export interface AppConfig {
auto_start: boolean
minimize_to_tray: boolean
notifications_enabled: boolean
theme: ThemeOption
}

export interface MemoryConfig {
provider: MemoryProviderKind
}

export interface Config {
capture: CaptureConfig
summary: SummaryConfig
app: AppConfig
memory: MemoryConfig
}

export interface ApiKeyStatus {
gemini_configured: boolean
supermemory_configured: boolean
}

### 6.2 Tauri command 封装

src/lib/tauri.ts 在 spec-01 的基础上追加：
ts
import { invoke } from '@tauri-apps/api/core'
import type { Config, ApiKeyStatus } from './types'

// 已有的 greet 保留

// Config
export async function getConfig(): Promise<Config> {
return invoke<Config>('get_config')
}

export async function setConfig(config: Config): Promise<void> {
return invoke<void>('set_config', { config })
}

// API Keys
export async function getApiKeyStatus(): Promise<ApiKeyStatus> {
return invoke<ApiKeyStatus>('get_api_key_status')
}

export async function saveGeminiApiKey(apiKey: string): Promise<void> {
return invoke<void>('save_gemini_api_key', { apiKey })
}

export async function saveSupermemoryApiKey(apiKey: string): Promise<void> {
return invoke<void>('save_supermemory_api_key', { apiKey })
}

export async function deleteGeminiApiKey(): Promise<void> {
return invoke<void>('delete_gemini_api_key')
}

export async function deleteSupermemoryApiKey(): Promise<void> {
return invoke<void>('delete_supermemory_api_key')
}

export async function testGeminiConnection(apiKey: string): Promise<void> {
return invoke<void>('test_gemini_connection', { apiKey })
}

export async function testSupermemoryConnection(apiKey: string): Promise<void> {
return invoke<void>('test_supermemory_connection', { apiKey })
}

### 6.3 安装额外 shadcn 组件

bash
pnpm dlx shadcn@latest add input label

### 6.4 Settings 页面骨架

src/pages/settings/settings-page.tsx：
tsx
import { useState } from 'react'
import { cn } from '@/lib/utils'
import { ApiKeysSection } from './sections/api-keys-section'

type Section = 'general' | 'api-keys' | 'storage' | 'privacy' | 'about'

const sections: { id: Section; label: string }[] = [
{ id: 'general', label: '常规' },
{ id: 'api-keys', label: 'API Keys' },
{ id: 'storage', label: '存储' },
{ id: 'privacy', label: '隐私' },
{ id: 'about', label: '关于' },
]

export function SettingsPage() {
const [active, setActive] = useState<Section>('api-keys')

return (
<div className="space-y-6">
<h1 className="text-xl font-semibold">配置</h1>
<div className="flex gap-8">
<nav className="w-32 shrink-0 space-y-1">
{sections.map((s) => (
<button
key={s.id}
onClick={() => setActive(s.id)}
className={cn(
'w-full text-left text-sm px-3 py-2 rounded-md transition-colors',
active === s.id
? 'bg-accent text-accent-foreground font-medium'
: 'text-muted-foreground hover:text-foreground hover:bg-accent/40'
)}
>
{s.label}
</button>
))}
</nav>
<div className="flex-1 min-w-0">
{active === 'api-keys' && <ApiKeysSection />}
{active !== 'api-keys' && (
<div className="rounded-xl border border-border bg-card p-6 text-sm text-muted-foreground">
{sections.find((s) => s.id === active)?.label} · 待 spec-11 实现
</div>
)}
</div>
</div>
</div>
)
}

### 6.5 ApiKeysSection 组件

src/pages/settings/sections/api-keys-section.tsx：
tsx
import { useState, useEffect } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { Loader2 } from 'lucide-react'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Button } from '@/components/ui/button'
import {
getApiKeyStatus,
saveGeminiApiKey,
saveSupermemoryApiKey,
testGeminiConnection,
testSupermemoryConnection,
} from '@/lib/tauri'

export function ApiKeysSection() {
const queryClient = useQueryClient()
const { data: status } = useQuery({
queryKey: ['api-key-status'],
queryFn: getApiKeyStatus,
})

const [geminiKey, setGeminiKey] = useState('')
const [supermemoryKey, setSupermemoryKey] = useState('')
const [testingGemini, setTestingGemini] = useState(false)
const [testingSm, setTestingSm] = useState(false)

// status 拿到后用占位符暗示已配置（不显示明文）
useEffect(() => {
if (status?.gemini_configured && !geminiKey) {
setGeminiKey('••••••••••••••••')
}
if (status?.supermemory_configured && !supermemoryKey) {
setSupermemoryKey('••••••••••••••••')
}
}, [status])

const saveGemini = useMutation({
mutationFn: saveGeminiApiKey,
onSuccess: () => {
toast.success('Gemini API key 已保存')
queryClient.invalidateQueries({ queryKey: ['api-key-status'] })
},
onError: (e: string) => toast.error(`保存失败: ${e}`),
})

const saveSm = useMutation({
mutationFn: saveSupermemoryApiKey,
onSuccess: () => {
toast.success('Supermemory API key 已保存')
queryClient.invalidateQueries({ queryKey: ['api-key-status'] })
},
onError: (e: string) => toast.error(`保存失败: ${e}`),
})

const handleTestGemini = async () => {
if (!geminiKey || geminiKey.startsWith('•')) {
toast.error('请先输入新的 API key')
return
}
setTestingGemini(true)
try {
await testGeminiConnection(geminiKey)
toast.success('Gemini 连接成功')
} catch (e) {
toast.error(`Gemini 连接失败: ${e}`)
} finally {
setTestingGemini(false)
}
}

const handleTestSm = async () => {
if (!supermemoryKey || supermemoryKey.startsWith('•')) {
toast.error('请先输入新的 API key')
return
}
setTestingSm(true)
try {
await testSupermemoryConnection(supermemoryKey)
toast.success('Supermemory 连接成功')
} catch (e) {
toast.error(`Supermemory 连接失败: ${e}`)
} finally {
setTestingSm(false)
}
}

return (
<div className="space-y-8 max-w-xl">
<div>
<h2 className="text-base font-medium mb-1">API Keys</h2>
<p className="text-xs text-muted-foreground">
所有 API key 加密存储在系统 keychain 中，不会出现在配置文件里
</p>
</div>

      {/* Gemini */}
      <div className="space-y-2">
        <Label>Gemini API Key</Label>
        <div className="flex gap-2">
          <Input
            type="password"
            value={geminiKey}
            onChange={(e) => setGeminiKey(e.target.value)}
            placeholder="AIza..."
            className="flex-1"
          />
          <Button
            variant="outline"
            onClick={handleTestGemini}
            disabled={testingGemini}
          >
            {testingGemini && <Loader2 className="w-3 h-3 mr-1 animate-spin" />}
            测试
          </Button>
          <Button
            onClick={() => saveGemini.mutate(geminiKey)}
            disabled={!geminiKey || geminiKey.startsWith('•') || saveGemini.isPending}
          >
            保存
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          用于截图总结。{' '}
          
            href="https://ai.google.dev/gemini-api/docs/api-key"
            className="underline hover:text-foreground"
          >
            获取 Gemini API key
          </a>
        </p>
      </div>

      {/* Supermemory */}
      <div className="space-y-2">
        <Label>Supermemory API Key</Label>
        <div className="flex gap-2">
          <Input
            type="password"
            value={supermemoryKey}
            onChange={(e) => setSupermemoryKey(e.target.value)}
            placeholder="sm_..."
            className="flex-1"
          />
          <Button
            variant="outline"
            onClick={handleTestSm}
            disabled={testingSm}
          >
            {testingSm && <Loader2 className="w-3 h-3 mr-1 animate-spin" />}
            测试
          </Button>
          <Button
            onClick={() => saveSm.mutate(supermemoryKey)}
            disabled={!supermemoryKey || supermemoryKey.startsWith('•') || saveSm.isPending}
          >
            保存
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          用于记忆存储。{' '}
          
            href="https://supermemory.ai/dashboard"
            className="underline hover:text-foreground"
          >
            获取 Supermemory API key
          </a>
        </p>
      </div>
    </div>
)
}
关键设计点：
- 已保存的 key 不从后端读出明文，前端只显示 •••••••• 作为"已配置"的视觉提示
- 用户必须输入新值才能再次测试或保存
- 测试和保存是两个独立动作，允许用户先测后保存
- 用 useMutation + useQuery + invalidateQueries 让保存后立刻刷新状态徽标

## 七、任务分解（给 Claude Code 的执行序列）
暂时无法在飞书文档外展示此内容

## 八、验收清单

-  cargo build 后端编译成功，无 warning
-  cargo test 跑过基本单元测试
-  启动 app，应用数据目录里出现 config.json
-  config.json 内容是 Config struct 的默认值，不包含任何 api_key 字段
-  进入「配置」页，「API Keys」是默认激活子项
-  看到两个 API key input 和对应的「测试」「保存」按钮
-  输入一个错误的 Gemini key，点测试，toast 显示失败原因
-  输入一个正确的 Gemini key，点测试，toast 显示成功
-  点保存，toast 显示成功，输入框变成 ••••••••
-  重启 app，进入配置页，看到输入框仍然是 ••••••••（说明 keychain 持久化生效）
-  macOS Keychain Access 应用能搜到 com.corivo.app 的两条记录
-  Supermemory 同上验证一遍
-  整个过程没有任何明文 key 出现在 config.json、devtools network 面板（除测试请求体外）、console 日志中

## 九、坑点预警

1. keyring crate 在 Linux CI 上需要 dbus-x11：本地 macOS 开发不会遇到，但 GitHub Actions 跑 Linux 编译时可能报错。先不管，等 CI 阶段再处理。
2. tauri-plugin-store 的 path 解析：默认存到 $APPDATA/Corivo/config.json（macOS 是 ~/Library/Application Support/com.corivo.app/）。第一次找不到时跑一下 app.path().app_data_dir() 打印路径确认。
3. Supermemory API endpoint 可能变：写代码前先用 curl -H "Authorization: Bearer YOUR_KEY" https://api.supermemory.ai/v3/... 跑一遍，确认 base URL、auth 方式、错误码。Supermemory 文档迭代快，trait 实现里的具体路径以你跑通的为准。
4. Gemini list models 也消耗配额：免费层有限制，频繁点测试可能触发 429。健康检查不会让你超限，但要意识到这点。
5. 前端 mask 显示的 bug：用户如果直接清空输入框，useEffect 会再次填回 ••••••••。要在 onChange 里做处理：清空后不再自动填回，让用户能真正清空重输。可以加一个 userEdited flag 控制。
6. •••••••• 长度固定：不要根据实际 key 长度变化显示不同数量的点，那会暗示 key 长度信息。固定 16 个就行。
7. 测试连接的 timeout：网络不好时 15 秒可能不够。给前端按钮一个 30 秒的超时兜底，否则 loading 会无限转。
8. set_config 不要存 key：再三确认 Config struct 里没有任何 api_key 字段。如果哪天有人手滑加进去，一次保存就会把 key 写进 config.json 明文。可以加一个 unit test 断言序列化后的 JSON 不含 "api_key" 字符串。

## 十、产出物

   完成 spec-02 后你应该有：
- 一套完整的 Rust 端配置 + 密钥基础设施
- 用户能在 UI 上配置并测试 Gemini / Supermemory 的 API key
- 安全的密钥存储（不落盘）
- MemoryProvider / LlmProvider 的 trait 已经定义，但只实现了 health_check
- 后续 spec-03 (memory provider 完整实现) / spec-04 (llm provider 完整实现) 可以无缝接续
