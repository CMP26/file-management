use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use nexalearn_backend::{
    assessment, config::Config, db, frontend, ingestion, llm, storage, videos, whisper, AppState,
};
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(
    paths(
        healthz,
        nexalearn_backend::llm::handler::get_llm_status,
        nexalearn_backend::videos::list_videos,
        nexalearn_backend::videos::get_video,
        nexalearn_backend::ingestion::handler::upload_video,
        nexalearn_backend::assessment::handler::get_video_questions,
        nexalearn_backend::assessment::handler::start_exam_attempt,
        nexalearn_backend::assessment::handler::submit_attempt,
        nexalearn_backend::assessment::handler::get_justification
    ),
    components(
        schemas(
            nexalearn_backend::models::UploadResponse,
            nexalearn_backend::models::VideoOverview,
            nexalearn_backend::models::VideoListResponse,
            nexalearn_backend::models::VideoDetailResponse,
            nexalearn_backend::models::LlmStatusResponse,
            nexalearn_backend::models::QuestionChoiceResponse,
            nexalearn_backend::models::QuestionResponse,
            nexalearn_backend::models::TopicQuestionGroupResponse,
            nexalearn_backend::models::QuestionsByVideoResponse,
            nexalearn_backend::models::StartExamRequest,
            nexalearn_backend::models::StartExamResponse,
            nexalearn_backend::models::SubmitAnswerInput,
            nexalearn_backend::models::SubmitAttemptRequest,
            nexalearn_backend::models::AttemptBreakdownItem,
            nexalearn_backend::models::SubmitAttemptResponse,
            nexalearn_backend::models::JustificationResponse
        )
    ),
    tags(
        (name = "Health", description = "Service health check"),
        (name = "LLM", description = "LLM connectivity checks"),
        (name = "Videos", description = "Video catalog and processing status"),
        (name = "Ingestion", description = "Video upload and processing"),
        (name = "Assessment", description = "Question retrieval, exam flow, grading, and justifications")
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
    let gemma = llm::gemma::GemmaClient::new(&config.gemma_base_url, &config.gemma_model);
    let whisper = whisper::client::WhisperClient::new(&config.whisper_url);

    let bind_addr = config.bind_addr.clone();
    let state = AppState::new(config, pool, storage, gemma, whisper);

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/llm/status", get(llm::handler::get_llm_status))
        .route("/api/videos", get(videos::list_videos))
        .route("/api/videos/:video_id", get(videos::get_video))
        .route("/api/videos/upload", post(ingestion::handler::upload_video))
        .route(
            "/api/videos/:video_id/questions",
            get(assessment::handler::get_video_questions),
        )
        .route(
            "/api/videos/:video_id/exams/start",
            post(assessment::handler::start_exam_attempt),
        )
        .route(
            "/api/exams/:attempt_id/submit",
            post(assessment::handler::submit_attempt),
        )
        .route(
            "/api/exams/:attempt_id/answers/:answer_id/justification",
            get(assessment::handler::get_justification),
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
