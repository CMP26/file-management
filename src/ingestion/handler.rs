use crate::{
    models::{MuxImportUploadUrlRequest, MuxImportUploadUrlResponse, UploadResponse},
    AppError, AppResult, AppState,
};
use axum::{
    extract::{Multipart, State},
    Json,
};
use reqwest::{header, Url};
use uuid::Uuid;

const MAX_REMOTE_MEDIA_BYTES: u64 = 1024 * 1024 * 1024;

#[utoipa::path(
    post,
    path = "/api/videos/upload",
    tag = "Ingestion",
    responses(
        (status = 200, description = "Video uploaded and ingestion queued", body = UploadResponse),
        (status = 400, description = "Bad request"),
        (status = 500, description = "Internal server error")
    )
)]

pub async fn upload_video(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> AppResult<Json<UploadResponse>> {
    let mut title: Option<String> = None;
    let mut course_id: Option<Uuid> = None;
    let mut video_bytes: Option<Vec<u8>> = None;
    let mut media_content_type = "application/octet-stream".to_string();
    let mut media_extension = "bin".to_string();

    while let Some(field) = multipart.next_field().await? {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "title" => title = Some(field.text().await?),
            "course_id" => {
                let raw_course_id = field.text().await?;
                course_id = Some(
                    Uuid::parse_str(raw_course_id.trim())
                        .map_err(|_| AppError::bad_request("course_id must be a UUID"))?,
                );
            }
            "file" => {
                if let Some(content_type) = field.content_type() {
                    media_content_type = content_type.to_string();
                }
                if let Some(file_name) = field.file_name() {
                    media_extension = extension_from_filename(file_name);
                }
                video_bytes = Some(field.bytes().await?.to_vec());
            }
            _ => {}
        }
    }

    let title = title.ok_or_else(|| AppError::bad_request("missing title field"))?;
    let course_id = course_id.ok_or_else(|| AppError::bad_request("missing course_id field"))?;
    let video_bytes = video_bytes.ok_or_else(|| AppError::bad_request("missing file field"))?;

    ensure_course_exists(&state, course_id).await?;

    let (video_id, _) = enqueue_video(
        &state,
        course_id,
        title,
        video_bytes,
        media_content_type,
        media_extension,
    )
    .await?;

    Ok(Json(UploadResponse {
        video_id,
        course_id,
        status: "pending".to_string(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/mux/import-upload-url",
    tag = "Mux",
    request_body = MuxImportUploadUrlRequest,
    responses(
        (status = 200, description = "Mux-hosted video fetched and ingestion queued", body = MuxImportUploadUrlResponse),
        (status = 400, description = "Bad request"),
        (status = 502, description = "Mux URL fetch failed"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn import_mux_upload_url(
    State(state): State<AppState>,
    Json(payload): Json<MuxImportUploadUrlRequest>,
) -> AppResult<Json<MuxImportUploadUrlResponse>> {
    let title = payload.title.trim();
    if title.is_empty() {
        return Err(AppError::bad_request("title cannot be empty"));
    }
    ensure_course_exists(&state, payload.course_id).await?;

    let url = Url::parse(payload.upload_url.trim())
        .map_err(|_| AppError::bad_request("upload_url must be a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::bad_request("upload_url must use http or https"));
    }

    let response = reqwest::Client::new()
        .get(url.clone())
        .send()
        .await
        .map_err(|error| AppError::external(format!("failed to fetch mux upload url: {error}")))?;
    let response = response.error_for_status().map_err(|error| {
        AppError::external(format!("mux upload url returned an error: {error}"))
    })?;

    if let Some(content_length) = response.content_length() {
        if content_length > MAX_REMOTE_MEDIA_BYTES {
            return Err(AppError::bad_request(format!(
                "remote media is larger than {} bytes",
                MAX_REMOTE_MEDIA_BYTES
            )));
        }
    }

    let media_content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let media_extension = payload
        .file_name
        .as_deref()
        .map(extension_from_filename)
        .or_else(|| extension_from_url(&url))
        .unwrap_or_else(|| extension_from_content_type(&media_content_type));
    let video_bytes = response
        .bytes()
        .await
        .map_err(|error| AppError::external(format!("failed to read mux upload bytes: {error}")))?;

    if video_bytes.len() as u64 > MAX_REMOTE_MEDIA_BYTES {
        return Err(AppError::bad_request(format!(
            "remote media is larger than {} bytes",
            MAX_REMOTE_MEDIA_BYTES
        )));
    }

    let (video_id, rustfs_key) = enqueue_video(
        &state,
        payload.course_id,
        title.to_string(),
        video_bytes.to_vec(),
        media_content_type,
        media_extension,
    )
    .await?;

    Ok(Json(MuxImportUploadUrlResponse {
        video_id,
        course_id: payload.course_id,
        status: "pending".to_string(),
        rustfs_key,
    }))
}

async fn enqueue_video(
    state: &AppState,
    course_id: Uuid,
    title: String,
    video_bytes: Vec<u8>,
    media_content_type: String,
    media_extension: String,
) -> AppResult<(Uuid, String)> {
    let video_id = Uuid::new_v4();
    let rustfs_key = format!("videos/{video_id}/original.{media_extension}");

    state
        .storage
        .upload(&rustfs_key, video_bytes, &media_content_type)
        .await?;

    sqlx::query(
        r#"
        INSERT INTO videos (id, course_id, title, rustfs_key, status)
        VALUES ($1, $2, $3, $4, 'pending')
        "#,
    )
    .bind(video_id)
    .bind(course_id)
    .bind(title)
    .bind(&rustfs_key)
    .execute(&state.pool)
    .await?;

    let worker_state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = crate::ingestion::worker::process_video(worker_state, video_id).await {
            tracing::error!(video_id = %video_id, error = %error, "ingestion worker failed");
        }
    });

    Ok((video_id, rustfs_key))
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

fn extension_from_filename(file_name: &str) -> String {
    file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| !extension.is_empty())
        .map(|extension| {
            extension
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .filter(|extension| !extension.is_empty())
        .unwrap_or_else(|| "bin".to_string())
}

fn extension_from_url(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .map(extension_from_filename)
        .filter(|extension| extension != "bin")
}

fn extension_from_content_type(content_type: &str) -> String {
    match content_type.split(';').next().unwrap_or_default().trim() {
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        "audio/wav" | "audio/x-wav" => "wav",
        _ => "bin",
    }
    .to_string()
}
