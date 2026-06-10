use crate::{config::Config, db::pool::Pool, llm::gemma::GemmaClient, storage::rustfs::RustFsClient, whisper::client::WhisperClient};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub pool: Pool,
    pub storage: RustFsClient,
    pub gemma: GemmaClient,
    pub whisper: WhisperClient,
}

impl AppState {
    pub fn new(config: Config, pool: Pool, storage: RustFsClient, gemma: GemmaClient, whisper: WhisperClient) -> Self {
        Self { config, pool, storage, gemma, whisper }
    }
}
