use crate::{models::UploadResponse, AppError, AppResult, AppState};
use axum::{
    extract::{Multipart, State},
    Json,
};
use uuid::Uuid;

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
    let mut video_bytes: Option<Vec<u8>> = None;
    let mut media_content_type = "application/octet-stream".to_string();
    let mut media_extension = "bin".to_string();

    while let Some(field) = multipart.next_field().await? {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "title" => title = Some(field.text().await?),
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
    let video_bytes = video_bytes.ok_or_else(|| AppError::bad_request("missing file field"))?;

    let video_id = Uuid::new_v4();
    let rustfs_key = format!("videos/{video_id}/original.{media_extension}");

    state
        .storage
        .upload(&rustfs_key, video_bytes, &media_content_type)
        .await?;

    sqlx::query(
        r#"
        INSERT INTO videos (id, title, rustfs_key, status)
        VALUES ($1, $2, $3, 'pending')
        "#,
    )
    .bind(video_id)
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

    Ok(Json(UploadResponse {
        video_id,
        status: "pending".to_string(),
    }))
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
