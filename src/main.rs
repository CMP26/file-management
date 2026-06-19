use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use nexalearn_backend::{
    assessment, chat, config::Config, courses, db, documents, embedding, frontend, ingestion, llm,
    storage, videos, whisper, AppState,
};
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(
    paths(
        healthz,
        nexalearn_backend::llm::handler::get_llm_status,
        nexalearn_backend::courses::list_courses,
        nexalearn_backend::courses::create_course,
        nexalearn_backend::courses::delete_course,
        nexalearn_backend::documents::upload_document,
        nexalearn_backend::documents::list_documents,
        nexalearn_backend::documents::get_document,
        nexalearn_backend::documents::delete_document,
        nexalearn_backend::documents::stream_document_events,
        nexalearn_backend::documents::get_document_file,
        nexalearn_backend::videos::list_videos,
        nexalearn_backend::videos::get_video,
        nexalearn_backend::videos::stream_video_events,
        nexalearn_backend::videos::delete_video,
        nexalearn_backend::videos::get_video_media,
        nexalearn_backend::videos::get_video_transcript,
        nexalearn_backend::videos::get_video_transcript_vtt,
        nexalearn_backend::ingestion::handler::upload_video,
        nexalearn_backend::ingestion::handler::import_mux_download_url,
        nexalearn_backend::assessment::handler::get_video_questions,
        nexalearn_backend::assessment::handler::get_course_random_questions,
        nexalearn_backend::assessment::handler::start_exam_attempt,
        nexalearn_backend::assessment::handler::get_attempt_status,
        nexalearn_backend::assessment::handler::stream_attempt_events,
        nexalearn_backend::assessment::handler::submit_attempt,
        nexalearn_backend::assessment::handler::get_justification,
        nexalearn_backend::assessment::handler::start_justification,
        nexalearn_backend::assessment::handler::get_justification_status,
        nexalearn_backend::assessment::handler::stream_justification_events,
        nexalearn_backend::chat::start_video_chat,
        nexalearn_backend::chat::send_chat_message,
        nexalearn_backend::chat::list_user_chats,
        nexalearn_backend::chat::get_user_chat,
        nexalearn_backend::chat::stream_user_chat_events,
        nexalearn_backend::chat::delete_user_chat
    ),
    components(
        schemas(
            nexalearn_backend::models::UploadResponse,
            nexalearn_backend::models::MuxImportDownloadUrlRequest,
            nexalearn_backend::models::MuxImportDownloadUrlResponse,
            nexalearn_backend::models::CourseResponse,
            nexalearn_backend::models::CourseListResponse,
            nexalearn_backend::models::CreateCourseRequest,
            nexalearn_backend::models::DeleteCourseResponse,
            nexalearn_backend::models::DocumentResponse,
            nexalearn_backend::models::DocumentListResponse,
            nexalearn_backend::models::DocumentUploadResponse,
            nexalearn_backend::models::DeleteDocumentResponse,
            nexalearn_backend::models::SourceVideoResponse,
            nexalearn_backend::models::VideoOverview,
            nexalearn_backend::models::VideoListResponse,
            nexalearn_backend::models::VideoTopicResponse,
            nexalearn_backend::models::VideoDetailResponse,
            nexalearn_backend::models::DeleteVideoResponse,
            nexalearn_backend::models::TranscriptSegmentResponse,
            nexalearn_backend::models::VideoTranscriptResponse,
            nexalearn_backend::models::LlmStatusResponse,
            nexalearn_backend::models::QuestionChoiceResponse,
            nexalearn_backend::models::QuestionResponse,
            nexalearn_backend::models::TopicQuestionGroupResponse,
            nexalearn_backend::models::QuestionsByVideoResponse,
            nexalearn_backend::models::CourseRandomQuestionResponse,
            nexalearn_backend::models::CourseRandomQuestionsResponse,
            nexalearn_backend::models::StartExamRequest,
            nexalearn_backend::models::StartExamResponse,
            nexalearn_backend::models::SubmitAnswerInput,
            nexalearn_backend::models::SubmitAttemptRequest,
            nexalearn_backend::models::AttemptBreakdownItem,
            nexalearn_backend::models::SubmitAttemptResponse,
            nexalearn_backend::models::AttemptAnswerStatusItem,
            nexalearn_backend::models::AttemptStatusResponse,
            nexalearn_backend::models::JustificationStatusResponse,
            nexalearn_backend::models::JustificationResponse,
            nexalearn_backend::models::TranscriptChatMessage,
            nexalearn_backend::models::StartTranscriptChatRequest,
            nexalearn_backend::models::TranscriptChatRequest,
            nexalearn_backend::models::TranscriptChatSource,
            nexalearn_backend::models::TranscriptChatResponse,
            nexalearn_backend::models::TranscriptChatMessageResponse,
            nexalearn_backend::models::TranscriptChatHistoryResponse,
            nexalearn_backend::models::UserChatConversationResponse,
            nexalearn_backend::models::UserChatListResponse,
            nexalearn_backend::models::DeleteChatResponse
        )
    ),
    tags(
        (name = "Health", description = "Service health check"),
        (name = "LLM", description = "LLM connectivity checks"),
        (name = "Courses", description = "Course catalog for grouping source videos"),
        (name = "Documents", description = "Course PDF upload, processing, and retrieval"),
        (name = "Videos", description = "Video catalog and processing status"),
        (name = "Ingestion", description = "Video upload and processing"),
        (name = "Mux", description = "Mux URL import integration"),
        (name = "Assessment", description = "Question retrieval, exam flow, grading, and justifications"),
        (name = "Chat", description = "Transcript-grounded chatbot responses")
    )
)]
struct ApiDoc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .compact()
        .init();

    let config = Config::from_env()?;
    let pool = db::create_pool(&config.database_url).await?;
    db::run_migrations(&pool).await?;

    let storage = storage::rustfs::RustFsClient::new(&config).await?;
    let gemma = llm::gemma::GemmaClient::new(
        &config.gemma_base_url,
        &config.gemma_model,
        config.gemma_max_concurrent_requests,
        config.gemma_request_timeout_seconds,
    );
    let embeddings =
        embedding::OllamaEmbeddingClient::new(&config.ollama_base_url, &config.embedding_model);
    let whisper = whisper::client::WhisperClient::new(&config.whisper_url);

    let bind_addr = config.bind_addr.clone();
    let state = AppState::new(config, pool, storage, gemma, embeddings, whisper);

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/llm/status", get(llm::handler::get_llm_status))
        .route(
            "/api/courses",
            get(courses::list_courses).post(courses::create_course),
        )
        .route(
            "/api/courses/:course_id",
            axum::routing::delete(courses::delete_course),
        )
        .route("/api/documents", get(documents::list_documents))
        .route("/api/documents/upload", post(documents::upload_document))
        .route(
            "/api/documents/:document_id",
            get(documents::get_document).delete(documents::delete_document),
        )
        .route(
            "/api/documents/:document_id/events",
            get(documents::stream_document_events),
        )
        .route(
            "/api/documents/:document_id/file",
            get(documents::get_document_file),
        )
        .route("/api/videos", get(videos::list_videos))
        .route(
            "/api/videos/:video_id",
            get(videos::get_video).delete(videos::delete_video),
        )
        .route(
            "/api/videos/:video_id/events",
            get(videos::stream_video_events),
        )
        .route("/api/videos/:video_id/media", get(videos::get_video_media))
        .route(
            "/api/videos/:video_id/transcript",
            get(videos::get_video_transcript),
        )
        .route(
            "/api/videos/:video_id/transcript.vtt",
            get(videos::get_video_transcript_vtt),
        )
        .route("/api/videos/:video_id/chats", post(chat::start_video_chat))
        .route(
            "/api/chats/:conversation_id/messages",
            post(chat::send_chat_message),
        )
        .route("/api/users/:user_id/chats", get(chat::list_user_chats))
        .route(
            "/api/users/:user_id/chats/:conversation_id",
            get(chat::get_user_chat).delete(chat::delete_user_chat),
        )
        .route(
            "/api/users/:user_id/chats/:conversation_id/events",
            get(chat::stream_user_chat_events),
        )
        .route("/api/videos/upload", post(ingestion::handler::upload_video))
        .route(
            "/api/mux/import-download-url",
            post(ingestion::handler::import_mux_download_url),
        )
        .route(
            "/api/mux/import-upload-url",
            post(ingestion::handler::import_mux_download_url),
        )
        .route(
            "/api/videos/:video_id/questions",
            get(assessment::handler::get_video_questions),
        )
        .route(
            "/api/courses/:course_id/questions/random",
            get(assessment::handler::get_course_random_questions),
        )
        .route(
            "/api/videos/:video_id/exams/start",
            post(assessment::handler::start_exam_attempt),
        )
        .route(
            "/api/exams/:attempt_id",
            get(assessment::handler::get_attempt_status),
        )
        .route(
            "/api/exams/:attempt_id/events",
            get(assessment::handler::stream_attempt_events),
        )
        .route(
            "/api/exams/:attempt_id/submit",
            post(assessment::handler::submit_attempt),
        )
        .route(
            "/api/exams/:attempt_id/answers/:answer_id/justification",
            get(assessment::handler::get_justification),
        )
        .route(
            "/api/exams/:attempt_id/answers/:answer_id/justification/start",
            post(assessment::handler::start_justification),
        )
        .route(
            "/api/exams/:attempt_id/answers/:answer_id/justification/status",
            get(assessment::handler::get_justification_status),
        )
        .route(
            "/api/exams/:attempt_id/answers/:answer_id/justification/events",
            get(assessment::handler::stream_justification_events),
        )
        .merge(frontend::router())
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .layer(DefaultBodyLimit::max(1024 * 1024 * 1024))
        .with_state(state);

    let bind_addr: std::net::SocketAddr = bind_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;

    tracing::info!("listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}

#[utoipa::path(
    get,
    path = "/healthz",
    tag = "Health",
    responses(
        (status = 200, description = "Service healthy", body = String)
    )
)]
async fn healthz() -> &'static str {
    "ok"
}
