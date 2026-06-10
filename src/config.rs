use crate::{AppError, AppResult};

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub rustfs_endpoint: String,
    pub rustfs_bucket: String,
    pub rustfs_access_key: String,
    pub rustfs_secret_key: String,
    pub gemma_base_url: String,
    pub gemma_model: String,
    pub whisper_url: String,
    pub tmp_dir: String,
    pub bind_addr: String,
}

impl Config {
    pub fn from_env() -> AppResult<Self> {
        Ok(Self {
            database_url: required("DATABASE_URL")?,
            rustfs_endpoint: required("RUSTFS_ENDPOINT")?,
            rustfs_bucket: env_or("RUSTFS_BUCKET", "nexalearn"),
            rustfs_access_key: env_or("RUSTFS_ACCESS_KEY", "minio"),
            rustfs_secret_key: env_or("RUSTFS_SECRET_KEY", "minio12345"),
                gemma_base_url: env_or("GEMMA_BASE_URL", "http://host.docker.internal:8100"),
            gemma_model: env_or("GEMMA_MODEL", "gemma3"),
            whisper_url: env_or("WHISPER_URL", "http://localhost:8000"),
            tmp_dir: env_or("TMP_DIR", "/tmp/nexalearn"),
            bind_addr: env_or("BIND_ADDR", "0.0.0.0:8080"),
        })
    }
}

fn required(name: &str) -> AppResult<String> {
    std::env::var(name).map_err(|_| AppError::bad_request(format!("missing environment variable {name}")))
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}
