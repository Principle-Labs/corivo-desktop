//! 编译期环境分流。
//!
//! 把所有 dev/prod 差异（API base、未来可能的 feature flag、
//! 调试开关等）集中在这一个文件，避免散落在各处。
//!
//! - `cargo build` / `tauri dev`              → debug → IS_DEV = true
//! - `cargo build --release` / `tauri build`  → release → IS_DEV = false
//!
//! The open-source build does not talk to Corivo-hosted services by
//! default. Forks can point cloud-capability implementations at their own
//! backend by setting `CORIVO_API_BASE` / `CORIVO_WEB_BASE`.
//!
//! 数据目录隔离不在这里 —— 那是通过 `tauri.dev.conf.json`
//! 用不同的 bundle identifier (`ai.corivo.desktop.dev` vs
//! `ai.corivo.desktop`) 来达成的，macOS 会按 identifier 自动派生
//! `~/Library/Application Support/<identifier>/`，无需手工拼路径。

/// True 时表示当前为开发构建（`debug_assertions` 开启）。
pub const IS_DEV: bool = cfg!(debug_assertions);

/// 编译期默认 API base。运行时仍然允许通过 `CORIVO_API_BASE`
/// 环境变量临时覆盖（见 `api_base()`）—— 这条留作单次调试用，
/// 避免每次切环境都要改代码重新编译。
pub const DEFAULT_API_BASE: &str = if IS_DEV {
    "http://localhost:8787"
} else {
    "http://localhost:8787"
};

/// 取当前生效的 API base。优先 `CORIVO_API_BASE` env，回退到
/// `DEFAULT_API_BASE`。空白字符串视为未设。
pub fn api_base() -> String {
    std::env::var("CORIVO_API_BASE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string())
}

/// 编译期默认 Web 站点 base。Google OAuth loopback 在拿到 code 之后
/// 会 302 把浏览器跳到 `${WEB_BASE}/oauth/success` 或 `/oauth/error`，
/// 这两个页面部署在 `apps/web` 里。
///
/// 用 `CORIVO_WEB_BASE` 在运行时覆盖（dev 切到自部署 web 时用）。
pub const DEFAULT_WEB_BASE: &str = if IS_DEV {
    "http://localhost:3000"
} else {
    "http://localhost:3000"
};

pub fn web_base() -> String {
    std::env::var("CORIVO_WEB_BASE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| DEFAULT_WEB_BASE.to_string())
}

/// 自定义 URL scheme，用于"返回 Corivo"按钮的 deep link 唤起。
/// dev / release 用不同的 scheme，避免一台机器上同时装了两份时
/// 互相唤起到错误的进程。
pub const DEEP_LINK_SCHEME: &str = if IS_DEV { "corivo-dev" } else { "corivo" };

/// 编译期烘焙的 Google OAuth client_id。
///
/// 通过 `option_env!` 读取构建期 `CORIVO_GOOGLE_CLIENT_ID`，运行时仍允许
/// 同名环境变量覆盖（dev 切环境用）。返回 None 时登录命令会直接报
/// "Google 登录未配置"，避免在没配 client 的二进制里走半截流程再失败。
///
/// 这个值 **不是密码**：Google 的 OAuth 模型把 desktop client 的
/// client_id 当公开值看待（任何人拆 .app 都能拿到），靠 PKCE +
/// 后端验签 ID Token 保证安全。所以 client_id 可以直接随 binary 发布；
/// 我们只是不写死在 git 里，方便切 dev/prod 不同的 client。
const COMPILED_GOOGLE_CLIENT_ID: Option<&str> = option_env!("CORIVO_GOOGLE_CLIENT_ID");

pub fn google_client_id() -> Option<String> {
    std::env::var("CORIVO_GOOGLE_CLIENT_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            COMPILED_GOOGLE_CLIENT_ID
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
        })
}

/// Google "Desktop app" client 的伪客户端密钥。
///
/// Google 的 [native-app 文档](https://developers.google.com/identity/protocols/oauth2/native-app)
/// 里明说 desktop client_secret "is not actually treated as a secret"
/// —— 它会随 binary 被反编译出来，真正的安全机制是 PKCE。token
/// exchange 时如果 OAuth client 注册成 "Web application" 类型必须传
/// secret；"Desktop app" 类型可省。两种 client 都兼容。
const COMPILED_GOOGLE_CLIENT_SECRET: Option<&str> = option_env!("CORIVO_GOOGLE_CLIENT_SECRET");

pub fn google_client_secret() -> Option<String> {
    std::env::var("CORIVO_GOOGLE_CLIENT_SECRET")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            COMPILED_GOOGLE_CLIENT_SECRET
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
        })
}

/// Compile-time baked Slack OAuth client_id / client_secret. Unlike
/// Google's desktop client, Slack's `client_secret` IS a real secret
/// — Slack v2 token exchange rejects requests without it. Both ends
/// must therefore be configured for the Slack connector to work, and
/// we don't ship public binaries with the secret baked in (devs run
/// against their own Slack app during Phase 1).
const COMPILED_SLACK_CLIENT_ID: Option<&str> = option_env!("CORIVO_SLACK_CLIENT_ID");
const COMPILED_SLACK_CLIENT_SECRET: Option<&str> = option_env!("CORIVO_SLACK_CLIENT_SECRET");

pub fn slack_client_id() -> Option<String> {
    std::env::var("CORIVO_SLACK_CLIENT_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            COMPILED_SLACK_CLIENT_ID
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
        })
}

pub fn slack_client_secret() -> Option<String> {
    std::env::var("CORIVO_SLACK_CLIENT_SECRET")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            COMPILED_SLACK_CLIENT_SECRET
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
        })
}
