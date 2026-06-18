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
    pub gemma_max_concurrent_requests: usize,
    pub gemma_request_timeout_seconds: u64,
    pub ollama_base_url: String,
    pub embedding_model: String,
    pub semantic_cache_threshold: f32,
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
            gemma_base_url: env_or("GEMMA_BASE_URL", "http://localhost:8100"),
            gemma_model: env_or("GEMMA_MODEL", "ggml-org/gemma-4-E4B-it-GGUF"),
            gemma_max_concurrent_requests: env_or_usize("GEMMA_MAX_CONCURRENT_REQUESTS", 2)?,
            gemma_request_timeout_seconds: env_or_u64("GEMMA_REQUEST_TIMEOUT_SECONDS", 300)?,
            ollama_base_url: env_or("OLLAMA_BASE_URL", "http://localhost:11434"),
            embedding_model: env_or("EMBEDDING_MODEL", "nomic-embed-text"),
            semantic_cache_threshold: env_or_f32("SEMANTIC_CACHE_THRESHOLD", 0.70)?,
            whisper_url: env_or("WHISPER_URL", "http://localhost:8000"),
            tmp_dir: env_or("TMP_DIR", "/tmp/nexalearn"),
            bind_addr: env_or("BIND_ADDR", "0.0.0.0:8080"),
        })
    }
}

fn env_or_f32(name: &str, default: f32) -> AppResult<f32> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<f32>()
            .map_err(|_| AppError::bad_request(format!("{name} must be a number")))
            .and_then(|value| {
                if (0.0..=1.0).contains(&value) {
                    Ok(value)
                } else {
                    Err(AppError::bad_request(format!(
                        "{name} must be between 0 and 1"
                    )))
                }
            }),
        Err(_) => Ok(default),
    }
}

fn required(name: &str) -> AppResult<String> {
    std::env::var(name)
        .map_err(|_| AppError::bad_request(format!("missing environment variable {name}")))
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_or_usize(name: &str, default: usize) -> AppResult<usize> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<usize>()
            .map_err(|_| AppError::bad_request(format!("{name} must be a positive integer")))
            .and_then(|value| {
                if value == 0 {
                    Err(AppError::bad_request(format!(
                        "{name} must be greater than 0"
                    )))
                } else {
                    Ok(value)
                }
            }),
        Err(_) => Ok(default),
    }
}

fn env_or_u64(name: &str, default: u64) -> AppResult<u64> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| AppError::bad_request(format!("{name} must be a positive integer")))
            .and_then(|value| {
                if value == 0 {
                    Err(AppError::bad_request(format!(
                        "{name} must be greater than 0"
                    )))
                } else {
                    Ok(value)
                }
            }),
        Err(_) => Ok(default),
    }
}
