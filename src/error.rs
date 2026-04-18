use thiserror::Error;

pub type CliResult<T> = Result<T, CliError>;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("config error: {0}")]
    Config(String),

    #[error("not configured: run `omd configure` first")]
    NotConfigured,

    #[error("not authenticated: run `omd auth login` first")]
    NotAuthenticated,

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("api error: {status} - {message}")]
    Api { status: u16, message: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("toml serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::NotConfigured | CliError::NotAuthenticated => 2,
            CliError::InvalidInput(_) => 64,
            CliError::NotFound(_) => 3,
            CliError::Api { status, .. } if *status == 401 || *status == 403 => 77,
            _ => 1,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            CliError::Config(_) => "config",
            CliError::NotConfigured => "not_configured",
            CliError::NotAuthenticated => "not_authenticated",
            CliError::Http(_) => "http",
            CliError::Api { .. } => "api",
            CliError::Io(_) => "io",
            CliError::Json(_) => "json",
            CliError::TomlParse(_) | CliError::TomlSer(_) => "toml",
            CliError::InvalidInput(_) => "invalid_input",
            CliError::NotFound(_) => "not_found",
            CliError::Other(_) => "other",
        }
    }
}
