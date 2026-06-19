use crate::{
    config::Config, db::pool::Pool, embedding::OllamaEmbeddingClient, llm::gemma::GemmaClient,
    storage::rustfs::RustFsClient, whisper::client::WhisperClient,
};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub pool: Pool,
    pub storage: RustFsClient,
    pub gemma: GemmaClient,
    pub embeddings: OllamaEmbeddingClient,
    pub whisper: WhisperClient,
    pub chat_events: broadcast::Sender<Uuid>,
    pub video_events: broadcast::Sender<Uuid>,
    pub document_events: broadcast::Sender<Uuid>,
    pub exam_events: broadcast::Sender<Uuid>,
    pub justification_events: broadcast::Sender<Uuid>,
}

impl AppState {
    pub fn new(
        config: Config,
        pool: Pool,
        storage: RustFsClient,
        gemma: GemmaClient,
        embeddings: OllamaEmbeddingClient,
        whisper: WhisperClient,
    ) -> Self {
        let (chat_events, _) = broadcast::channel(256);
        let (video_events, _) = broadcast::channel(256);
        let (document_events, _) = broadcast::channel(256);
        let (exam_events, _) = broadcast::channel(256);
        let (justification_events, _) = broadcast::channel(256);
        Self {
            config,
            pool,
            storage,
            gemma,
            embeddings,
            whisper,
            chat_events,
            video_events,
            document_events,
            exam_events,
            justification_events,
        }
    }
}
