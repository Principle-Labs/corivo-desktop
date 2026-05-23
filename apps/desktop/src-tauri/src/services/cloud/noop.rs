//! Open-source / no-cloud defaults for every capability trait.
//!
//! Every method here is either:
//!   * a `FeatureUnavailable` error (the operation has no meaningful
//!     offline interpretation — sign-in, top-ups, OAuth connectors,
//!     remote model directory refresh, …), or
//!   * a thoughtful local-only fallback (an empty list, an unmanaged
//!     `Capabilities` snapshot, the user's own Anthropic key for the
//!     agent sidecar).
//!
//! `is_available()` / `is_managed()` always return `false` so the
//! frontend `Capabilities` probe gets a faithful picture.
//!
//! These impls are zero-sized unit structs so cloning the
//! `CloudServices` bundle on every command call is essentially free.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::{CorivoError, Result};

use super::auth::{AuthService, AuthStatus, EmailVerifyResult, RequestEmailCodeOutcome};
use super::billing::{BillingMe, BillingService};
use super::connectors::{
    ComposioConnectionLink, ComposioConnectionsResponse, ConnectorSummary, ConnectorsService,
};
use super::models::{ModelDirectory, ModelsService};
use super::session::{CloudAgentCreds, CloudSessionService};
use super::telemetry::TelemetryService;
use super::updater_policy::{UpdatePolicy, UpdaterPolicyService};

fn unavailable(feature: &'static str, message: &str) -> CorivoError {
    CorivoError::FeatureUnavailable {
        feature,
        message: message.to_string(),
    }
}

// ----------------------------------------------------------------------------
// Auth
// ----------------------------------------------------------------------------

pub struct NoopAuthService;

#[async_trait]
impl AuthService for NoopAuthService {
    fn is_available(&self) -> bool {
        false
    }

    async fn has_session(&self) -> Result<bool> {
        Ok(false)
    }

    async fn status(&self) -> Result<AuthStatus> {
        // OSS 用户没有"账号"概念，但 AuthStatus 是前端用来判断是否要把
        // 路由送到 /login 的核心 IPC。返回一个 logged_in=false +
        // api_base 留空的 status，前端会自然走"无登录"分支。
        Ok(AuthStatus {
            logged_in: false,
            label: None,
            access_expires_at: None,
            refresh_expires_at: None,
            api_base: String::new(),
            email: None,
            name: None,
            avatar_url: None,
        })
    }

    async fn login_google(&self) -> Result<AuthStatus> {
        Err(unavailable(
            "auth",
            "开源构建不支持云端登录；请通过设置面板配置 Anthropic API key 直接使用。",
        ))
    }

    async fn request_email_code(&self, _email: String) -> Result<RequestEmailCodeOutcome> {
        Err(unavailable("auth", "开源构建不支持邮箱验证码登录。"))
    }

    async fn login_email(&self, _email: String, _code: String) -> Result<EmailVerifyResult> {
        Err(unavailable("auth", "开源构建不支持邮箱验证码登录。"))
    }

    async fn logout(&self) -> Result<()> {
        // 没有登录态，所以"登出"是 no-op，幂等返回 Ok。
        Ok(())
    }

    async fn refresh(&self) -> Result<AuthStatus> {
        self.status().await
    }
}

// ----------------------------------------------------------------------------
// Billing
// ----------------------------------------------------------------------------

pub struct NoopBillingService;

#[async_trait]
impl BillingService for NoopBillingService {
    fn is_available(&self) -> bool {
        false
    }

    async fn me(&self) -> Result<BillingMe> {
        Err(unavailable(
            "billing",
            "开源构建不提供托管计费；直接使用自带的 LLM key 即可。",
        ))
    }

    async fn start_checkout_url(&self, _amount_usd: f64) -> Result<String> {
        Err(unavailable("billing", "开源构建未启用计费。"))
    }
}

// ----------------------------------------------------------------------------
// Models
// ----------------------------------------------------------------------------

pub struct NoopModelsService;

#[async_trait]
impl ModelsService for NoopModelsService {
    fn is_available(&self) -> bool {
        false
    }

    async fn get_available(&self) -> Result<ModelDirectory> {
        // 不返回错误：Settings UI 在 mount 时无脑调这个，错误会让整页
        // 显示"加载失败"。Task #5 会把它换成"读取 Config 里用户自带
        // key 后合成一个单条目 directory"；当前 stub 先返回错误让上层
        // 在迁移时显式适配。
        Err(unavailable(
            "models_directory",
            "开源构建没有托管模型目录；在设置中填入自带 API key 即可使用。",
        ))
    }

    async fn refresh(&self) -> Result<ModelDirectory> {
        Err(unavailable(
            "models_directory",
            "开源构建没有托管模型目录可供刷新。",
        ))
    }

    async fn set_active_alias(&self, _alias: String) -> Result<ModelDirectory> {
        Err(unavailable(
            "models_directory",
            "开源构建只有一个模型来源（自带 key），无需切换。",
        ))
    }
}

// ----------------------------------------------------------------------------
// Cloud session
// ----------------------------------------------------------------------------

pub struct NoopCloudSessionService;

#[async_trait]
impl CloudSessionService for NoopCloudSessionService {
    fn has_session(&self) -> Result<bool> {
        Ok(false)
    }

    async fn fetch_agent_creds(&self) -> Result<CloudAgentCreds> {
        Err(unavailable(
            "session",
            "开源构建没有 Corivo 云端会话；请在设置中使用自带 API key。",
        ))
    }

    async fn refresh_agent_creds_now(&self) -> Result<()> {
        Err(unavailable(
            "session",
            "开源构建没有 Corivo 云端会话可刷新。",
        ))
    }

    async fn refresh_model_directory(
        &self,
        _catalog: Arc<crate::services::model_catalog::ModelCatalog>,
    ) -> Result<ModelDirectory> {
        Err(unavailable(
            "models_directory",
            "开源构建没有托管模型目录可供刷新。",
        ))
    }

    async fn set_active_model_alias(
        &self,
        _catalog: Arc<crate::services::model_catalog::ModelCatalog>,
        _alias: String,
    ) -> Result<ModelDirectory> {
        Err(unavailable(
            "models_directory",
            "开源构建只有一个模型来源（自带 key），无需切换。",
        ))
    }

    async fn ensure_model_alias_synced(
        &self,
        _catalog: Arc<crate::services::model_catalog::ModelCatalog>,
        _alias: String,
    ) -> Result<()> {
        Err(unavailable(
            "models_directory",
            "开源构建没有远端模型路由可同步。",
        ))
    }
}

// ----------------------------------------------------------------------------
// Connectors
// ----------------------------------------------------------------------------

pub struct NoopConnectorsService;

#[async_trait]
impl ConnectorsService for NoopConnectorsService {
    fn is_available(&self) -> bool {
        false
    }

    async fn list(&self) -> Result<Vec<ConnectorSummary>> {
        // 返回空列表（不是错误）：Settings → Integrations 整页在
        // capability=false 时根本不会渲染，但 list 偶然被调到时也不应
        // 把整个设置页打爆。
        Ok(Vec::new())
    }

    async fn enable(&self, _id: String) -> Result<ConnectorSummary> {
        Err(unavailable("connectors", "开源构建未启用第三方连接器。"))
    }

    async fn disable(&self, _id: String) -> Result<()> {
        Err(unavailable("connectors", "开源构建未启用第三方连接器。"))
    }

    async fn connect(&self, _id: String) -> Result<ConnectorSummary> {
        Err(unavailable("connectors", "开源构建未启用第三方连接器。"))
    }

    async fn provider_connect(
        &self,
        _provider: String,
        _connector_ids: Vec<String>,
    ) -> Result<Vec<ConnectorSummary>> {
        Err(unavailable("connectors", "开源构建未启用第三方连接器。"))
    }

    async fn install_mcp(&self, _id: String) -> Result<ConnectorSummary> {
        Err(unavailable("connectors", "开源构建未启用第三方连接器。"))
    }

    async fn disconnect(&self, _id: String) -> Result<ConnectorSummary> {
        Err(unavailable("connectors", "开源构建未启用第三方连接器。"))
    }

    async fn composio_list_connections(&self) -> Result<ComposioConnectionsResponse> {
        Err(unavailable("connectors", "开源构建未启用 Composio 网关。"))
    }

    async fn composio_create_connection_link(
        &self,
        _toolkit_slug: String,
        _redirect_url: Option<String>,
    ) -> Result<ComposioConnectionLink> {
        Err(unavailable("connectors", "开源构建未启用 Composio 网关。"))
    }

    async fn composio_disconnect(&self, _connection_id: String) -> Result<()> {
        Err(unavailable("connectors", "开源构建未启用 Composio 网关。"))
    }
}

// ----------------------------------------------------------------------------
// Telemetry
// ----------------------------------------------------------------------------

pub struct NoopTelemetryService;

#[async_trait]
impl TelemetryService for NoopTelemetryService {
    fn is_available(&self) -> bool {
        false
    }

    async fn track_event(
        &self,
        _name: String,
        _properties: Option<BTreeMap<String, Value>>,
    ) -> Result<()> {
        // 显式 no-op，不报错。前端 track_event 永远 fire-and-forget，
        // 让它在 OSS 构建里"成功地什么也不做"是最稳的契约。
        Ok(())
    }
}

// ----------------------------------------------------------------------------
// Updater policy
// ----------------------------------------------------------------------------

pub struct NoopUpdaterPolicyService;

#[async_trait]
impl UpdaterPolicyService for NoopUpdaterPolicyService {
    fn is_managed(&self) -> bool {
        false
    }

    async fn fetch_policy(&self) -> Result<Option<UpdatePolicy>> {
        // OSS 不注册 Tauri updater，也没有 minVersion 门槛。返回 None
        // 让 UI 跳过"是否强制更新"判定。
        Ok(None)
    }
}

// Agent credentials live in `Config.exec_agent` (auth_mode + byok_*)
// and the runner consumes them through `CorivoAuth`. The open-source
// build's behaviour (auto-default to Byok, force the user to paste a
// key in Settings) is enforced by the cfg-gated `Default` impl on
// `ExecAgentAuthMode` rather than a trait noop here.
