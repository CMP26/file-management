use crate::{
    config::Config, db::pool::Pool, llm::gemma::GemmaClient, storage::rustfs::RustFsClient,
    whisper::client::WhisperClient,
};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub pool: Pool,
    pub storage: RustFsClient,
    pub gemma: GemmaClient,
    pub whisper: WhisperClient,
    pub chat_events: broadcast::Sender<Uuid>,
    pub video_events: broadcast::Sender<Uuid>,
    pub exam_events: broadcast::Sender<Uuid>,
    pub justification_events: broadcast::Sender<Uuid>,
}

impl AppState {
    pub fn new(
        config: Config,
        pool: Pool,
        storage: RustFsClient,
        gemma: GemmaClient,
        whisper: WhisperClient,
    ) -> Self {
        let (chat_events, _) = broadcast::channel(256);
        let (video_events, _) = broadcast::channel(256);
        let (exam_events, _) = broadcast::channel(256);
        let (justification_events, _) = broadcast::channel(256);
        Self {
            config,
            pool,
            storage,
            gemma,
            whisper,
            chat_events,
            video_events,
            exam_events,
            justification_events,
        }
    }
}
