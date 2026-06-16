use crate::{
    ingestion::audio::create_playback_video,
    models::{
        DeleteVideoResponse, TranscriptSegmentResponse, VideoDetailResponse, VideoListResponse,
        VideoOverview, VideoTopicResponse, VideoTranscriptResponse,
    },
    AppError, AppResult, AppState,
};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use std::{convert::Infallible, path::PathBuf};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

#[derive(Debug, FromRow)]
struct VideoOverviewRow {
    id: Uuid,
    course_id: Uuid,
    course_title: String,
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
    Ok(Json(video_detail_response(&state, video_id).await?))
}

#[utoipa::path(
    get,
    path = "/api/videos/{video_id}/events",
    tag = "Videos",
    params(
        ("video_id" = Uuid, Path, description = "Video id")
    ),
    responses(
        (status = 200, description = "Server-sent video processing updates"),
        (status = 404, description = "Video not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn stream_video_events(
    State(state): State<AppState>,
    Path(video_id): Path<Uuid>,
) -> AppResult<Sse<ReceiverStream<Result<Event, Infallible>>>> {
    let mut video_events = state.video_events.subscribe();
    let initial = video_detail_response(&state, video_id).await?;
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);
    let worker_state = state.clone();

    tokio::spawn(async move {
        let _ = send_video_event(&tx, &initial).await;
        if is_terminal_video_status(&initial.video.status) {
            return;
        }

        loop {
            match video_events.recv().await {
                Ok(changed_video_id) if changed_video_id == video_id => {
                    match video_detail_response(&worker_state, video_id).await {
                        Ok(snapshot) => {
                            let terminal = is_terminal_video_status(&snapshot.video.status);
                            if send_video_event(&tx, &snapshot).await.is_err() {
                                return;
                            }
                            if terminal {
                                return;
                            }
                        }
                        Err(error) => {
                            let _ = tx
                                .send(Ok(Event::default().event("error").data(error.to_string())))
                                .await;
                            return;
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = tx
                        .send(Ok(Event::default().event("error").data(error.to_string())))
                        .await;
                    return;
                }
            }
        }
    });

    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

async fn video_detail_response(state: &AppState, video_id: Uuid) -> AppResult<VideoDetailResponse> {
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

    let topics = sqlx::query_as::<_, (Uuid, String, f64, f64, i32)>(
        "SELECT id, label, start_s, end_s, seq_order FROM topics WHERE video_id = $1 ORDER BY seq_order",
    )
    .bind(video_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(id, label, start_s, end_s, seq_order)| VideoTopicResponse {
        id,
        label,
        start_s,
        end_s,
        seq_order,
    })
    .collect();

    Ok(VideoDetailResponse {
        video: row.into(),
        topics,
        summary,
        transcript_preview,
    })
}

async fn send_video_event(
    tx: &mpsc::Sender<Result<Event, Infallible>>,
    snapshot: &VideoDetailResponse,
) -> Result<(), mpsc::error::SendError<Result<Event, Infallible>>> {
    let data = serde_json::to_string(snapshot).unwrap_or_else(|error| {
        format!(
            "{{\"error\":\"failed to serialize video snapshot\",\"message\":\"{}\"}}",
            error
        )
    });
    tx.send(Ok(Event::default().event("video").data(data)))
        .await
}

fn is_terminal_video_status(status: &str) -> bool {
    matches!(status, "ready" | "failed")
}

#[utoipa::path(
    delete,
    path = "/api/videos/{video_id}",
    tag = "Videos",
    params(
        ("video_id" = Uuid, Path, description = "Video id")
    ),
    responses(
        (status = 200, description = "Video deleted", body = DeleteVideoResponse),
        (status = 404, description = "Video not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_video(
    State(state): State<AppState>,
    Path(video_id): Path<Uuid>,
) -> AppResult<Json<DeleteVideoResponse>> {
    let rustfs_key = sqlx::query_scalar::<_, String>("SELECT rustfs_key FROM videos WHERE id = $1")
        .bind(video_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found(format!("video {video_id} was not found")))?;

    sqlx::query("DELETE FROM videos WHERE id = $1")
        .bind(video_id)
        .execute(&state.pool)
        .await?;

    for key in [
        rustfs_key,
        format!("videos/{video_id}/playback.mp4"),
        format!("videos/{video_id}/transcript.txt"),
        format!("videos/{video_id}/transcript.vtt"),
    ] {
        if let Err(error) = state.storage.delete(&key).await {
            tracing::warn!(video_id = %video_id, key = %key, error = %error, "failed to delete object");
        }
    }

    Ok(Json(DeleteVideoResponse {
        video_id,
        deleted: true,
    }))
}

#[utoipa::path(
    get,
    path = "/api/videos/{video_id}/media",
    tag = "Videos",
    params(
        ("video_id" = Uuid, Path, description = "Video id")
    ),
    responses(
        (status = 200, description = "Original uploaded media"),
        (status = 404, description = "Video not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_video_media(
    State(state): State<AppState>,
    Path(video_id): Path<Uuid>,
    request_headers: HeaderMap,
) -> AppResult<Response> {
    let rustfs_key = sqlx::query_scalar::<_, String>("SELECT rustfs_key FROM videos WHERE id = $1")
        .bind(video_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found(format!("video {video_id} was not found")))?;

    let (media_key, bytes) = load_playback_or_original(&state, video_id, &rustfs_key).await?;
    let content_type = media_content_type(&media_key, &bytes);
    Ok(media_response(
        bytes,
        content_type,
        request_headers.get(header::RANGE),
    ))
}

#[utoipa::path(
    get,
    path = "/api/videos/{video_id}/transcript.vtt",
    tag = "Videos",
    params(
        ("video_id" = Uuid, Path, description = "Video id")
    ),
    responses(
        (status = 200, description = "VTT transcript captions"),
        (status = 404, description = "Video not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_video_transcript_vtt(
    State(state): State<AppState>,
    Path(video_id): Path<Uuid>,
) -> AppResult<impl IntoResponse> {
    ensure_video_exists(&state, video_id).await?;
    let bytes = state
        .storage
        .download(&format!("videos/{video_id}/transcript.vtt"))
        .await?;
    Ok((media_headers("text/vtt; charset=utf-8"), bytes))
}

#[utoipa::path(
    get,
    path = "/api/videos/{video_id}/transcript",
    tag = "Videos",
    params(
        ("video_id" = Uuid, Path, description = "Video id")
    ),
    responses(
        (status = 200, description = "Transcript text and timestamped segments", body = VideoTranscriptResponse),
        (status = 404, description = "Video not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_video_transcript(
    State(state): State<AppState>,
    Path(video_id): Path<Uuid>,
) -> AppResult<Json<VideoTranscriptResponse>> {
    ensure_video_exists(&state, video_id).await?;

    let transcript = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, full_text FROM transcripts WHERE video_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(video_id)
    .fetch_optional(&state.pool)
    .await?;

    let Some((transcript_id, full_text)) = transcript else {
        return Ok(Json(VideoTranscriptResponse {
            video_id,
            full_text: None,
            segments: Vec::new(),
        }));
    };

    let segments = sqlx::query_as::<_, (i32, f64, f64, String)>(
        r#"
        SELECT seq_index, start_s, end_s, text
        FROM transcript_segments
        WHERE transcript_id = $1
        ORDER BY seq_index
        "#,
    )
    .bind(transcript_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(
        |(seq_index, start_s, end_s, text)| TranscriptSegmentResponse {
            seq_index,
            start_s,
            end_s,
            text,
        },
    )
    .collect();

    Ok(Json(VideoTranscriptResponse {
        video_id,
        full_text: Some(full_text),
        segments,
    }))
}

async fn load_playback_or_original(
    state: &AppState,
    video_id: Uuid,
    original_key: &str,
) -> AppResult<(String, Vec<u8>)> {
    let playback_key = format!("videos/{video_id}/playback.mp4");
    if let Ok(bytes) = state.storage.download(&playback_key).await {
        return Ok((playback_key, bytes));
    }

    let original_bytes = state.storage.download(original_key).await?;
    let tmp_dir = PathBuf::from(&state.config.tmp_dir);
    tokio::fs::create_dir_all(&tmp_dir).await?;

    let input_extension = original_key
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| !extension.is_empty())
        .unwrap_or("bin");
    let input_path = tmp_dir.join(format!("{video_id}.playback-source.{input_extension}"));
    let playback_path = tmp_dir.join(format!("{video_id}.playback-on-demand.mp4"));

    tokio::fs::write(&input_path, &original_bytes).await?;
    let result = match create_playback_video(&input_path, &playback_path).await {
        Ok(()) => {
            let playback_bytes = tokio::fs::read(&playback_path).await?;
            state
                .storage
                .upload(&playback_key, playback_bytes.clone(), "video/mp4")
                .await?;
            Ok((playback_key, playback_bytes))
        }
        Err(error) => {
            tracing::warn!(video_id = %video_id, error = %error, "failed to create on-demand playback video");
            Ok((original_key.to_string(), original_bytes))
        }
    };

    let _ = tokio::fs::remove_file(input_path).await;
    let _ = tokio::fs::remove_file(playback_path).await;

    result
}

fn media_response(
    bytes: Vec<u8>,
    content_type: &'static str,
    range: Option<&HeaderValue>,
) -> Response {
    let len = bytes.len();
    if len == 0 {
        return (media_headers(content_type), bytes).into_response();
    }

    if let Some(range) = range.and_then(|value| value.to_str().ok()) {
        if let Some((start, end)) = parse_byte_range(range, len) {
            let chunk = bytes[start..=end].to_vec();
            let mut headers = media_headers(content_type);
            headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            headers.insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes {start}-{end}/{len}"))
                    .unwrap_or_else(|_| HeaderValue::from_static("bytes */*")),
            );
            headers.insert(
                header::CONTENT_LENGTH,
                HeaderValue::from_str(&chunk.len().to_string())
                    .unwrap_or_else(|_| HeaderValue::from_static("0")),
            );
            return (StatusCode::PARTIAL_CONTENT, headers, chunk).into_response();
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes */{len}")).unwrap(),
        );
        return (StatusCode::RANGE_NOT_SATISFIABLE, headers).into_response();
    }

    let mut headers = media_headers(content_type);
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&len.to_string()).unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    (headers, bytes).into_response()
}

fn parse_byte_range(range: &str, len: usize) -> Option<(usize, usize)> {
    let range = range.strip_prefix("bytes=")?;
    let (start, end) = range.split_once('-')?;

    if start.is_empty() {
        let suffix_len = end.parse::<usize>().ok()?;
        if suffix_len == 0 {
            return None;
        }
        let start = len.saturating_sub(suffix_len);
        return Some((start, len - 1));
    }

    let start = start.parse::<usize>().ok()?;
    if start >= len {
        return None;
    }

    let end = if end.is_empty() {
        len - 1
    } else {
        end.parse::<usize>().ok()?.min(len - 1)
    };

    if start > end {
        None
    } else {
        Some((start, end))
    }
}

fn media_headers(content_type: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers
}

fn media_content_type(key: &str, bytes: &[u8]) -> &'static str {
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        return match brand {
            b"qt  " => "video/quicktime",
            b"M4A " | b"M4B " => "audio/mp4",
            _ => "video/mp4",
        };
    }

    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return "video/webm";
    }

    if bytes.starts_with(b"OggS") {
        return "video/ogg";
    }

    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"AVI " {
        return "video/x-msvideo";
    }

    if bytes.starts_with(&[0x00, 0x00, 0x01, 0xBA]) {
        return "video/mpeg";
    }

    match key
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
    {
        Some(extension) if extension == "mp4" || extension == "m4v" => "video/mp4",
        Some(extension) if extension == "mov" => "video/quicktime",
        Some(extension) if extension == "webm" => "video/webm",
        Some(extension) if extension == "ogv" || extension == "ogg" => "video/ogg",
        Some(extension) if extension == "avi" => "video/x-msvideo",
        Some(extension) if extension == "mp3" => "audio/mpeg",
        Some(extension) if extension == "m4a" => "audio/mp4",
        Some(extension) if extension == "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

async fn ensure_video_exists(state: &AppState, video_id: Uuid) -> AppResult<()> {
    let exists = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM videos WHERE id = $1)")
        .bind(video_id)
        .fetch_one(&state.pool)
        .await?;

    if exists {
        Ok(())
    } else {
        Err(AppError::not_found(format!(
            "video {video_id} was not found"
        )))
    }
}

fn video_overview_sql(filter_video_id: bool) -> &'static str {
    match filter_video_id {
        true => {
            r#"
            SELECT
                v.id,
                v.course_id,
                c.title AS course_title,
                v.title,
                v.duration_s,
                v.status,
                v.error_msg,
                v.created_at,
                COUNT(DISTINCT t.id)::BIGINT AS topic_count,
                COUNT(DISTINCT q.id)::BIGINT AS question_count,
                EXISTS(SELECT 1 FROM summaries s WHERE s.video_id = v.id) AS has_summary
            FROM videos v
            JOIN courses c ON c.id = v.course_id
            LEFT JOIN topics t ON t.video_id = v.id
            LEFT JOIN questions q ON q.video_id = v.id
            WHERE v.id = $1
            GROUP BY v.id, c.id
            "#
        }
        false => {
            r#"
            SELECT
                v.id,
                v.course_id,
                c.title AS course_title,
                v.title,
                v.duration_s,
                v.status,
                v.error_msg,
                v.created_at,
                COUNT(DISTINCT t.id)::BIGINT AS topic_count,
                COUNT(DISTINCT q.id)::BIGINT AS question_count,
                EXISTS(SELECT 1 FROM summaries s WHERE s.video_id = v.id) AS has_summary
            FROM videos v
            JOIN courses c ON c.id = v.course_id
            LEFT JOIN topics t ON t.video_id = v.id
            LEFT JOIN questions q ON q.video_id = v.id
            GROUP BY v.id, c.id
            ORDER BY v.created_at DESC
            "#
        }
    }
}

impl From<VideoOverviewRow> for VideoOverview {
    fn from(row: VideoOverviewRow) -> Self {
        Self {
            id: row.id,
            course_id: row.course_id,
            course_title: row.course_title,
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
