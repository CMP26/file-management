use crate::{
    document_processing::{
        infer_document_recovery_stage, prepare_document_recovery, process_document_from_stage,
        DocumentProcessStage,
    },
    ingestion::worker::{
        infer_video_recovery_stage, prepare_video_recovery, process_video_from_stage,
        VideoProcessStage,
    },
    models::{RecoverUploadRequest, RecoverUploadsResponse, RecoveredUploadItem},
    AppError, AppResult, AppState,
};
use axum::{
    extract::{Path, State},
    Json,
};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryMode {
    Resume,
    Full,
}

impl RecoveryMode {
    fn parse(value: Option<&str>) -> AppResult<Self> {
        match value
            .unwrap_or("resume")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "resume" => Ok(Self::Resume),
            "full" => Ok(Self::Full),
            other => Err(AppError::bad_request(format!(
                "mode must be 'resume' or 'full', got {other:?}"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Resume => "resume",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, FromRow)]
struct RecoverableVideoRow {
    id: Uuid,
    title: String,
    status: String,
}

#[derive(Debug, FromRow)]
struct RecoverableDocumentRow {
    id: Uuid,
    title: String,
    status: String,
}

#[utoipa::path(
    post,
    path = "/api/courses/{course_id}/recover",
    tag = "Recovery",
    request_body = RecoverUploadRequest,
    responses(
        (status = 200, description = "Queued failed or incomplete course uploads for recovery", body = RecoverUploadsResponse),
        (status = 400, description = "Invalid recovery mode or stage"),
        (status = 404, description = "Course not found")
    )
)]
pub async fn recover_course_uploads(
    State(state): State<AppState>,
    Path(course_id): Path<Uuid>,
    Json(payload): Json<RecoverUploadRequest>,
) -> AppResult<Json<RecoverUploadsResponse>> {
    ensure_course_exists(&state, course_id).await?;
    let mode = RecoveryMode::parse(payload.mode.as_deref())?;
    if payload
        .stage
        .as_deref()
        .is_some_and(|stage| !stage.trim().is_empty())
    {
        return Err(AppError::bad_request(
            "stage override is only supported for single video or document recovery",
        ));
    }
    let include_ready = payload.include_ready.unwrap_or(false);

    let videos = sqlx::query_as::<_, RecoverableVideoRow>(
        "SELECT id, title, status FROM videos WHERE course_id = $1 ORDER BY created_at ASC",
    )
    .bind(course_id)
    .fetch_all(&state.pool)
    .await?;
    let documents = sqlx::query_as::<_, RecoverableDocumentRow>(
        "SELECT id, title, status FROM documents WHERE course_id = $1 ORDER BY created_at ASC",
    )
    .bind(course_id)
    .fetch_all(&state.pool)
    .await?;

    let mut queued = Vec::new();
    let mut skipped = Vec::new();

    for video in videos {
        if video.status == "ready" && !include_ready {
            skipped.push(video_item(&video, "ready"));
            continue;
        }
        let stage = match mode {
            RecoveryMode::Full => VideoProcessStage::ExtractingAudio,
            RecoveryMode::Resume => {
                infer_video_recovery_stage(&state, video.id, &video.status).await?
            }
        };
        prepare_video_recovery(&state, video.id, stage).await?;
        spawn_video_recovery(state.clone(), video.id, stage);
        queued.push(video_item(&video, stage.as_status()));
    }

    for document in documents {
        if document.status == "ready" && !include_ready {
            skipped.push(document_item(&document, "ready"));
            continue;
        }
        let stage = match mode {
            RecoveryMode::Full => DocumentProcessStage::Extracting,
            RecoveryMode::Resume => {
                infer_document_recovery_stage(&state, document.id, &document.status).await?
            }
        };
        prepare_document_recovery(&state, document.id, stage).await?;
        spawn_document_recovery(state.clone(), document.id, stage);
        queued.push(document_item(&document, stage.as_status()));
    }

    Ok(Json(RecoverUploadsResponse {
        course_id: Some(course_id),
        mode: mode.as_str().to_string(),
        queued,
        skipped,
    }))
}

#[utoipa::path(
    post,
    path = "/api/videos/{video_id}/recover",
    tag = "Recovery",
    request_body = RecoverUploadRequest,
    responses(
        (status = 200, description = "Queued one lesson for recovery", body = RecoverUploadsResponse),
        (status = 400, description = "Invalid recovery mode or stage"),
        (status = 404, description = "Video not found")
    )
)]
pub async fn recover_video_upload(
    State(state): State<AppState>,
    Path(video_id): Path<Uuid>,
    Json(payload): Json<RecoverUploadRequest>,
) -> AppResult<Json<RecoverUploadsResponse>> {
    let mode = RecoveryMode::parse(payload.mode.as_deref())?;
    let video = sqlx::query_as::<_, RecoverableVideoRow>(
        "SELECT id, title, status FROM videos WHERE id = $1",
    )
    .bind(video_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found(format!("video {video_id} was not found")))?;

    if video.status == "ready" && !payload.include_ready.unwrap_or(false) {
        return Ok(Json(RecoverUploadsResponse {
            course_id: None,
            mode: mode.as_str().to_string(),
            queued: Vec::new(),
            skipped: vec![video_item(&video, "ready")],
        }));
    }

    let stage = if let Some(stage) = payload
        .stage
        .as_deref()
        .filter(|stage| !stage.trim().is_empty())
    {
        parse_video_stage(stage)?
    } else if mode == RecoveryMode::Full {
        VideoProcessStage::ExtractingAudio
    } else {
        infer_video_recovery_stage(&state, video.id, &video.status).await?
    };
    prepare_video_recovery(&state, video.id, stage).await?;
    spawn_video_recovery(state, video.id, stage);

    Ok(Json(RecoverUploadsResponse {
        course_id: None,
        mode: mode.as_str().to_string(),
        queued: vec![video_item(&video, stage.as_status())],
        skipped: Vec::new(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/documents/{document_id}/recover",
    tag = "Recovery",
    request_body = RecoverUploadRequest,
    responses(
        (status = 200, description = "Queued one document for recovery", body = RecoverUploadsResponse),
        (status = 400, description = "Invalid recovery mode or stage"),
        (status = 404, description = "Document not found")
    )
)]
pub async fn recover_document_upload(
    State(state): State<AppState>,
    Path(document_id): Path<Uuid>,
    Json(payload): Json<RecoverUploadRequest>,
) -> AppResult<Json<RecoverUploadsResponse>> {
    let mode = RecoveryMode::parse(payload.mode.as_deref())?;
    let document = sqlx::query_as::<_, RecoverableDocumentRow>(
        "SELECT id, title, status FROM documents WHERE id = $1",
    )
    .bind(document_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found(format!("document {document_id} was not found")))?;

    if document.status == "ready" && !payload.include_ready.unwrap_or(false) {
        return Ok(Json(RecoverUploadsResponse {
            course_id: None,
            mode: mode.as_str().to_string(),
            queued: Vec::new(),
            skipped: vec![document_item(&document, "ready")],
        }));
    }

    let stage = if let Some(stage) = payload
        .stage
        .as_deref()
        .filter(|stage| !stage.trim().is_empty())
    {
        parse_document_stage(stage)?
    } else if mode == RecoveryMode::Full {
        DocumentProcessStage::Extracting
    } else {
        infer_document_recovery_stage(&state, document.id, &document.status).await?
    };
    prepare_document_recovery(&state, document.id, stage).await?;
    spawn_document_recovery(state, document.id, stage);

    Ok(Json(RecoverUploadsResponse {
        course_id: None,
        mode: mode.as_str().to_string(),
        queued: vec![document_item(&document, stage.as_status())],
        skipped: Vec::new(),
    }))
}

fn parse_video_stage(value: &str) -> AppResult<VideoProcessStage> {
    VideoProcessStage::parse(value.trim()).ok_or_else(|| {
        AppError::bad_request(
            "video stage must be one of extracting_audio, transcribing, labeling_topics, generating_questions, summarizing",
        )
    })
}

fn parse_document_stage(value: &str) -> AppResult<DocumentProcessStage> {
    DocumentProcessStage::parse(value.trim()).ok_or_else(|| {
        AppError::bad_request("document stage must be one of extracting or embedding")
    })
}

async fn ensure_course_exists(state: &AppState, course_id: Uuid) -> AppResult<()> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM courses WHERE id = $1)")
            .bind(course_id)
            .fetch_one(&state.pool)
            .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::not_found(format!(
            "course {course_id} was not found"
        )))
    }
}

fn spawn_video_recovery(state: AppState, video_id: Uuid, stage: VideoProcessStage) {
    tokio::spawn(async move {
        if let Err(error) = process_video_from_stage(state, video_id, stage).await {
            tracing::error!(video_id = %video_id, stage = stage.as_status(), error = %error, "video recovery worker failed");
        }
    });
}

fn spawn_document_recovery(state: AppState, document_id: Uuid, stage: DocumentProcessStage) {
    tokio::spawn(async move {
        if let Err(error) = process_document_from_stage(state, document_id, stage).await {
            tracing::error!(document_id = %document_id, stage = stage.as_status(), error = %error, "document recovery worker failed");
        }
    });
}

fn video_item(row: &RecoverableVideoRow, resume_stage: &str) -> RecoveredUploadItem {
    RecoveredUploadItem {
        id: row.id,
        kind: "video".to_string(),
        title: row.title.clone(),
        previous_status: row.status.clone(),
        resume_stage: resume_stage.to_string(),
    }
}

fn document_item(row: &RecoverableDocumentRow, resume_stage: &str) -> RecoveredUploadItem {
    RecoveredUploadItem {
        id: row.id,
        kind: "document".to_string(),
        title: row.title.clone(),
        previous_status: row.status.clone(),
        resume_stage: resume_stage.to_string(),
    }
}
