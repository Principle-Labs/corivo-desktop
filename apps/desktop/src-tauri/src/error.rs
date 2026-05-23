use thiserror::Error;

#[derive(Debug, Error)]
pub enum CorivoError {
    #[error("config error: {0}")]
    Config(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("LLM error: {0}")]
    Llm(String),
    #[error("LLM quota exceeded: {0}")]
    QuotaExceeded(String),
    #[error("LLM safety blocked: {0}")]
    SafetyBlocked(String),
    #[error("LLM invalid response: {0}")]
    InvalidResponse(String),
    #[error("internal: {0}")]
    Internal(String),
    /// Server rejected the cloud auth attempt. `email` carries the
    /// rejected Google account when the server already had it on hand,
    /// so the login UI can name it back to the user.
    #[error("auth denied: {message}")]
    AuthDenied {
        message: String,
        email: Option<String>,
    },
    /// 当前构建没有启用某个 cloud capability（例如开源版调用了 auth /
    /// billing / connectors 命令）。`feature` 是稳定的英文标识符
    /// （"auth" / "billing" / "models_directory" / "connectors" /
    /// "telemetry" / "managed_updater"），前端可以做精确分支；详细
    /// 中文文案放在 `message` 里，给 fallback toast 用。
    #[error("feature unavailable: {feature} ({message})")]
    FeatureUnavailable {
        feature: &'static str,
        message: String,
    },
}

impl CorivoError {
    pub fn user_message(&self) -> String {
        self.to_string()
    }
}

impl From<CorivoError> for String {
    fn from(error: CorivoError) -> Self {
        error.user_message()
    }
}

impl From<reqwest::Error> for CorivoError {
    fn from(error: reqwest::Error) -> Self {
        Self::Network(error.to_string())
    }
}

impl From<serde_json::Error> for CorivoError {
    fn from(error: serde_json::Error) -> Self {
        Self::Internal(format!("json: {error}"))
    }
}

impl From<rusqlite::Error> for CorivoError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Internal(format!("sqlite: {error}"))
    }
}

impl From<r2d2::Error> for CorivoError {
    fn from(error: r2d2::Error) -> Self {
        Self::Internal(format!("database pool: {error}"))
    }
}

impl From<std::io::Error> for CorivoError {
    fn from(error: std::io::Error) -> Self {
        Self::Internal(format!("io: {error}"))
    }
}

pub type Result<T> = std::result::Result<T, CorivoError>;
