use crate::{
    models::{VideoDetailResponse, VideoListResponse, VideoOverview},
    AppError, AppResult, AppState,
};
use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow)]
struct VideoOverviewRow {
    id: Uuid,
    title: String,
    duration_s: Option<i32>,
    status: String,
    error_msg: Option<String>,
    created_at: DateTime<Utc>,
    topic_count: i64,
    question_count: i64,
    has_summary: bool,
}

#[utoipa::path(
    get,
    path = "/api/videos",
    tag = "Videos",
    responses(
        (status = 200, description = "Uploaded videos with processing status", body = VideoListResponse),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_videos(State(state): State<AppState>) -> AppResult<Json<VideoListResponse>> {
    let rows: Vec<VideoOverviewRow> = sqlx::query_as(video_overview_sql(false))
        .fetch_all(&state.pool)
        .await?;

    Ok(Json(VideoListResponse {
        videos: rows.into_iter().map(VideoOverview::from).collect(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/videos/{video_id}",
    tag = "Videos",
    params(
        ("video_id" = Uuid, Path, description = "Video id")
    ),
    responses(
        (status = 200, description = "Video status and generated artifacts", body = VideoDetailResponse),
        (status = 404, description = "Video not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_video(
    State(state): State<AppState>,
    Path(video_id): Path<Uuid>,
) -> AppResult<Json<VideoDetailResponse>> {
    let row: VideoOverviewRow = sqlx::query_as(video_overview_sql(true))
        .bind(video_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found(format!("video {video_id} was not found")))?;

    let summary = sqlx::query_scalar::<_, String>(
        "SELECT content FROM summaries WHERE video_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(video_id)
    .fetch_optional(&state.pool)
    .await?;

    let transcript_preview = sqlx::query_scalar::<_, String>(
        "SELECT left(full_text, 1200) FROM transcripts WHERE video_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(video_id)
    .fetch_optional(&state.pool)
    .await?;

    Ok(Json(VideoDetailResponse {
        video: row.into(),
        summary,
        transcript_preview,
    }))
}

fn video_overview_sql(filter_video_id: bool) -> &'static str {
    match filter_video_id {
        true => {
            r#"
            SELECT
                v.id,
                v.title,
                v.duration_s,
                v.status,
                v.error_msg,
                v.created_at,
                COUNT(DISTINCT t.id)::BIGINT AS topic_count,
                COUNT(DISTINCT q.id)::BIGINT AS question_count,
                EXISTS(SELECT 1 FROM summaries s WHERE s.video_id = v.id) AS has_summary
            FROM videos v
            LEFT JOIN topics t ON t.video_id = v.id
            LEFT JOIN questions q ON q.video_id = v.id
            WHERE v.id = $1
            GROUP BY v.id
            "#
        }
        false => {
            r#"
            SELECT
                v.id,
                v.title,
                v.duration_s,
                v.status,
                v.error_msg,
                v.created_at,
                COUNT(DISTINCT t.id)::BIGINT AS topic_count,
                COUNT(DISTINCT q.id)::BIGINT AS question_count,
                EXISTS(SELECT 1 FROM summaries s WHERE s.video_id = v.id) AS has_summary
            FROM videos v
            LEFT JOIN topics t ON t.video_id = v.id
            LEFT JOIN questions q ON q.video_id = v.id
            GROUP BY v.id
            ORDER BY v.created_at DESC
            "#
        }
    }
}

impl From<VideoOverviewRow> for VideoOverview {
    fn from(row: VideoOverviewRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            duration_s: row.duration_s,
            status: row.status,
            error_msg: row.error_msg,
            created_at: row.created_at,
            topic_count: row.topic_count,
            question_count: row.question_count,
            has_summary: row.has_summary,
        }
    }
}
