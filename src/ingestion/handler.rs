use crate::{models::UploadResponse, AppError, AppResult, AppState};
use axum::{extract::{Multipart, State}, Json};
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

pub async fn upload_video(State(state): State<AppState>, mut multipart: Multipart) -> AppResult<Json<UploadResponse>> {
    let mut title: Option<String> = None;
    let mut video_bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart.next_field().await? {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "title" => title = Some(field.text().await?),
            "file" => video_bytes = Some(field.bytes().await?.to_vec()),
            _ => {}
        }
    }

    let title = title.ok_or_else(|| AppError::bad_request("missing title field"))?;
    let video_bytes = video_bytes.ok_or_else(|| AppError::bad_request("missing file field"))?;

    let video_id = Uuid::new_v4();
    let rustfs_key = format!("videos/{video_id}/original.mp4");

    state.storage.upload(&rustfs_key, video_bytes, "video/mp4").await?;

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
