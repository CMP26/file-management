use crate::{
    models::{
        DeleteChatResponse, StartTranscriptChatRequest, TranscriptChatHistoryResponse,
        TranscriptChatMessage, TranscriptChatMessageResponse, TranscriptChatRequest,
        TranscriptChatResponse, TranscriptChatSource, UserChatConversationResponse,
        UserChatListResponse,
    },
    AppError, AppResult, AppState,
};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct TranscriptSegment {
    seq_index: i32,
    start_s: f64,
    end_s: f64,
    text: String,
}

#[derive(Debug)]
struct ChatMessageRow {
    id: Uuid,
    role: String,
    content: String,
    sources_json: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug)]
struct ConversationContext {
    user_id: Uuid,
    video_id: Uuid,
    name: String,
    is_waiting: bool,
    video_title: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct ChatListFilters {
    pub video_id: Option<Uuid>,
}

#[utoipa::path(
    post,
    path = "/api/videos/{video_id}/chats",
    tag = "Chat",
    params(
        ("video_id" = Uuid, Path, description = "Video id")
    ),
    request_body = StartTranscriptChatRequest,
    responses(
        (status = 200, description = "Started a named transcript chat", body = TranscriptChatHistoryResponse),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Video not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn start_video_chat(
    State(state): State<AppState>,
    Path(video_id): Path<Uuid>,
    Json(payload): Json<StartTranscriptChatRequest>,
) -> AppResult<Json<TranscriptChatHistoryResponse>> {
    let video_title = video_title(&state, video_id).await?;
    let name = chat_name(payload.name.as_deref())?;
    let conversation_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO chat_conversations (user_id, video_id, name)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(payload.user_id)
    .bind(video_id)
    .bind(&name)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(TranscriptChatHistoryResponse {
        user_id: payload.user_id,
        video_id,
        video_title,
        conversation_id,
        name,
        is_waiting: false,
        messages: Vec::new(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/chats/{conversation_id}/messages",
    tag = "Chat",
    params(
        ("conversation_id" = Uuid, Path, description = "Chat conversation id")
    ),
    request_body = TranscriptChatRequest,
    responses(
        (status = 200, description = "Transcript-grounded chat response", body = TranscriptChatResponse),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Chat not found"),
        (status = 502, description = "LLM service error")
    )
)]
pub async fn send_chat_message(
    State(state): State<AppState>,
    Path(conversation_id): Path<Uuid>,
    Json(payload): Json<TranscriptChatRequest>,
) -> AppResult<Json<TranscriptChatResponse>> {
    let message = payload.message.trim();
    if message.is_empty() {
        return Err(AppError::bad_request("message cannot be empty"));
    }
    if message.chars().count() > 2_000 {
        return Err(AppError::bad_request(
            "message cannot be longer than 2000 characters",
        ));
    }

    let conversation = conversation_context(&state, payload.user_id, conversation_id).await?;
    let stored_history = load_chat_messages(&state, conversation_id).await?;
    let prompt_history = if stored_history.is_empty() {
        payload.history.clone()
    } else {
        stored_history
            .iter()
            .rev()
            .take(8)
            .rev()
            .map(|message| TranscriptChatMessage {
                role: message.role.clone(),
                content: message.content.clone(),
            })
            .collect()
    };

    let user_message_id =
        insert_chat_message(&state, conversation_id, "user", message, &[]).await?;
    set_conversation_waiting(&state, conversation_id, true).await?;

    let (answer, sources) =
        match generate_chat_answer(&state, &conversation, message, &prompt_history).await {
            Ok(result) => result,
            Err(error) => {
                set_conversation_waiting(&state, conversation_id, false).await?;
                return Err(error);
            }
        };

    let assistant_message_id =
        match insert_chat_message(&state, conversation_id, "assistant", &answer, &sources).await {
            Ok(message_id) => message_id,
            Err(error) => {
                set_conversation_waiting(&state, conversation_id, false).await?;
                return Err(error);
            }
        };
    set_conversation_waiting(&state, conversation_id, false).await?;

    Ok(Json(TranscriptChatResponse {
        conversation_id,
        video_id: conversation.video_id,
        name: conversation.name,
        is_waiting: false,
        user_message_id,
        assistant_message_id,
        answer,
        sources,
    }))
}

async fn generate_chat_answer(
    state: &AppState,
    conversation: &ConversationContext,
    message: &str,
    prompt_history: &[TranscriptChatMessage],
) -> AppResult<(String, Vec<TranscriptChatSource>)> {
    let transcript_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM transcripts WHERE video_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(conversation.video_id)
    .fetch_optional(&state.pool)
    .await?;

    let summary = sqlx::query_scalar::<_, String>(
        "SELECT content FROM summaries WHERE video_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(conversation.video_id)
    .fetch_optional(&state.pool)
    .await?;

    let segments = match transcript_id {
        Some(transcript_id) => load_transcript_segments(state, transcript_id).await?,
        None => Vec::new(),
    };

    let selected_segments = select_relevant_segments(message, prompt_history, &segments, 14);
    let prompt = transcript_chat_prompt(
        &conversation.video_title,
        summary.as_deref(),
        message,
        prompt_history,
        &selected_segments,
    );
    let answer = state
        .gemma
        .generate(&prompt)
        .await
        .map_err(|error| AppError::external(error.to_string()))?
        .trim()
        .to_string();
    let sources = selected_segments
        .into_iter()
        .map(|segment| TranscriptChatSource {
            seq_index: segment.seq_index,
            start_s: segment.start_s,
            end_s: segment.end_s,
            text: segment.text,
        })
        .collect();

    Ok((answer, sources))
}

#[utoipa::path(
    get,
    path = "/api/users/{user_id}/chats",
    tag = "Chat",
    params(
        ("user_id" = Uuid, Path, description = "User id"),
        ("video_id" = Option<Uuid>, Query, description = "Optional video id filter")
    ),
    responses(
        (status = 200, description = "Saved transcript chat conversations for this user", body = UserChatListResponse),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_user_chats(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Query(filters): Query<ChatListFilters>,
) -> AppResult<Json<UserChatListResponse>> {
    let chats = match filters.video_id {
        Some(video_id) => query_user_chats(&state, user_id, Some(video_id)).await?,
        None => query_user_chats(&state, user_id, None).await?,
    };

    Ok(Json(UserChatListResponse { user_id, chats }))
}

#[utoipa::path(
    get,
    path = "/api/users/{user_id}/chats/{conversation_id}",
    tag = "Chat",
    params(
        ("user_id" = Uuid, Path, description = "User id"),
        ("conversation_id" = Uuid, Path, description = "Chat conversation id")
    ),
    responses(
        (status = 200, description = "Saved transcript chat with its name, context, and messages", body = TranscriptChatHistoryResponse),
        (status = 404, description = "Chat not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_user_chat(
    State(state): State<AppState>,
    Path((user_id, conversation_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<TranscriptChatHistoryResponse>> {
    let conversation = conversation_context(&state, user_id, conversation_id).await?;
    let messages = load_chat_messages(&state, conversation_id)
        .await?
        .into_iter()
        .map(message_response_from_row)
        .collect();

    Ok(Json(TranscriptChatHistoryResponse {
        user_id: conversation.user_id,
        video_id: conversation.video_id,
        video_title: conversation.video_title,
        conversation_id,
        name: conversation.name,
        is_waiting: conversation.is_waiting,
        messages,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/users/{user_id}/chats/{conversation_id}",
    tag = "Chat",
    params(
        ("user_id" = Uuid, Path, description = "User id"),
        ("conversation_id" = Uuid, Path, description = "Chat conversation id")
    ),
    responses(
        (status = 200, description = "Deleted chat", body = DeleteChatResponse),
        (status = 404, description = "Chat not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_user_chat(
    State(state): State<AppState>,
    Path((user_id, conversation_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<DeleteChatResponse>> {
    let result = sqlx::query("DELETE FROM chat_conversations WHERE id = $1 AND user_id = $2")
        .bind(conversation_id)
        .bind(user_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::not_found(format!(
            "chat {conversation_id} was not found for user {user_id}"
        )));
    }

    Ok(Json(DeleteChatResponse {
        conversation_id,
        deleted: true,
    }))
}

async fn video_title(state: &AppState, video_id: Uuid) -> AppResult<String> {
    sqlx::query_scalar::<_, String>("SELECT title FROM videos WHERE id = $1")
        .bind(video_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found(format!("video {video_id} was not found")))
}

async fn conversation_context(
    state: &AppState,
    user_id: Uuid,
    conversation_id: Uuid,
) -> AppResult<ConversationContext> {
    sqlx::query_as::<_, (Uuid, Uuid, Uuid, String, bool, String)>(
        r#"
        SELECT c.id, c.user_id, c.video_id, c.name, c.is_waiting, v.title
        FROM chat_conversations c
        JOIN videos v ON v.id = c.video_id
        WHERE c.id = $1 AND c.user_id = $2
        "#,
    )
    .bind(conversation_id)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .map(
        |(_, user_id, video_id, name, is_waiting, video_title)| ConversationContext {
            user_id,
            video_id,
            name,
            is_waiting,
            video_title,
        },
    )
    .ok_or_else(|| AppError::not_found(format!("chat {conversation_id} was not found")))
}

async fn query_user_chats(
    state: &AppState,
    user_id: Uuid,
    video_id: Option<Uuid>,
) -> AppResult<Vec<UserChatConversationResponse>> {
    let mut query = String::from(
        r#"
        SELECT c.id, c.user_id, c.video_id, v.title, c.name, c.is_waiting, c.created_at, c.updated_at, count(m.id)::bigint AS message_count
        FROM chat_conversations c
        JOIN videos v ON v.id = c.video_id
        LEFT JOIN chat_messages m ON m.conversation_id = c.id
        WHERE c.user_id = $1
        "#,
    );

    if video_id.is_some() {
        query.push_str(" AND c.video_id = $2");
    }

    query.push_str(
        r#"
        GROUP BY c.id, c.user_id, c.video_id, v.title, c.name, c.is_waiting, c.created_at, c.updated_at
        ORDER BY c.updated_at DESC
        "#,
    );

    let rows = if let Some(video_id) = video_id {
        sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                Uuid,
                String,
                String,
                bool,
                DateTime<Utc>,
                DateTime<Utc>,
                i64,
            ),
        >(&query)
        .bind(user_id)
        .bind(video_id)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                Uuid,
                String,
                String,
                bool,
                DateTime<Utc>,
                DateTime<Utc>,
                i64,
            ),
        >(&query)
        .bind(user_id)
        .fetch_all(&state.pool)
        .await?
    };

    Ok(rows
        .into_iter()
        .map(
            |(
                conversation_id,
                user_id,
                video_id,
                video_title,
                name,
                is_waiting,
                created_at,
                updated_at,
                message_count,
            )| UserChatConversationResponse {
                conversation_id,
                user_id,
                video_id,
                video_title,
                name,
                is_waiting,
                created_at,
                updated_at,
                message_count,
            },
        )
        .collect())
}

async fn set_conversation_waiting(
    state: &AppState,
    conversation_id: Uuid,
    is_waiting: bool,
) -> AppResult<()> {
    sqlx::query("UPDATE chat_conversations SET is_waiting = $1, updated_at = now() WHERE id = $2")
        .bind(is_waiting)
        .bind(conversation_id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

async fn insert_chat_message(
    state: &AppState,
    conversation_id: Uuid,
    role: &str,
    content: &str,
    sources: &[TranscriptChatSource],
) -> AppResult<Uuid> {
    let sources_json = if sources.is_empty() {
        None
    } else {
        Some(serde_json::to_string(sources)?)
    };

    let message_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO chat_messages (conversation_id, role, content, sources_json)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(conversation_id)
    .bind(role)
    .bind(content)
    .bind(sources_json)
    .fetch_one(&state.pool)
    .await?;

    Ok(message_id)
}

async fn load_chat_messages(
    state: &AppState,
    conversation_id: Uuid,
) -> AppResult<Vec<ChatMessageRow>> {
    let rows = sqlx::query_as::<_, (Uuid, String, String, Option<String>, DateTime<Utc>)>(
        r#"
        SELECT id, role, content, sources_json, created_at
        FROM chat_messages
        WHERE conversation_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(conversation_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(
        |(id, role, content, sources_json, created_at)| ChatMessageRow {
            id,
            role,
            content,
            sources_json,
            created_at,
        },
    )
    .collect();

    Ok(rows)
}

async fn load_transcript_segments(
    state: &AppState,
    transcript_id: Uuid,
) -> AppResult<Vec<TranscriptSegment>> {
    Ok(sqlx::query_as::<_, (i32, f64, f64, String)>(
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
    .map(|(seq_index, start_s, end_s, text)| TranscriptSegment {
        seq_index,
        start_s,
        end_s,
        text,
    })
    .collect())
}

fn message_response_from_row(row: ChatMessageRow) -> TranscriptChatMessageResponse {
    TranscriptChatMessageResponse {
        id: row.id,
        role: row.role,
        content: row.content,
        sources: row
            .sources_json
            .as_deref()
            .and_then(|value| serde_json::from_str(value).ok())
            .unwrap_or_default(),
        created_at: row.created_at,
    }
}

fn chat_name(value: Option<&str>) -> AppResult<String> {
    let name = value.unwrap_or("Untitled chat").trim();
    if name.is_empty() {
        return Err(AppError::bad_request("chat name cannot be empty"));
    }
    if name.chars().count() > 120 {
        return Err(AppError::bad_request(
            "chat name cannot be longer than 120 characters",
        ));
    }
    Ok(name.to_string())
}

fn select_relevant_segments(
    message: &str,
    history: &[TranscriptChatMessage],
    segments: &[TranscriptSegment],
    limit: usize,
) -> Vec<TranscriptSegment> {
    let mut query = message.to_string();
    for chat_message in history.iter().rev().take(4) {
        query.push(' ');
        query.push_str(&chat_message.content);
    }

    let query_terms = tokenize(&query);
    let mut scored = segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let score = tokenize(&segment.text).intersection(&query_terms).count();
            (index, score)
        })
        .collect::<Vec<_>>();

    scored.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut selected_indexes = scored
        .into_iter()
        .filter(|(_, score)| *score > 0)
        .take(limit)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    if selected_indexes.is_empty() {
        selected_indexes = (0..segments.len().min(limit)).collect();
    }

    selected_indexes.sort_unstable();
    selected_indexes
        .into_iter()
        .map(|index| segments[index].clone())
        .collect()
}

fn tokenize(value: &str) -> HashSet<String> {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|term| term.len() > 2)
        .collect()
}

fn transcript_chat_prompt(
    video_title: &str,
    summary: Option<&str>,
    message: &str,
    history: &[TranscriptChatMessage],
    segments: &[TranscriptSegment],
) -> String {
    let mut prompt = String::from(
        "You are NexaLearn's learning chat assistant.\n\
        Use the provided video transcript excerpts when they are relevant, and cite those transcript-backed claims inline like [3:25].\n\
        You may also answer general questions that go beyond the video using your broader knowledge.\n\
        When an answer relies on outside knowledge rather than the transcript, say so briefly and avoid inventing video-specific details.\n\
        If the learner asks about the video and the excerpts do not contain enough information, say what is missing and then offer any helpful general context.\n\
        Be concise and helpful.\n\n",
    );

    prompt.push_str(&format!("Video title: {video_title}\n"));
    if let Some(summary) = summary {
        prompt.push_str("Video summary:\n");
        prompt.push_str(&truncate_chars(summary, 1_500));
        prompt.push_str("\n\n");
    }

    if !history.is_empty() {
        prompt.push_str("Current chat history:\n");
        for chat_message in history.iter().rev().take(8).rev() {
            let role = if chat_message.role == "assistant" {
                "assistant"
            } else {
                "user"
            };
            prompt.push_str(role);
            prompt.push_str(": ");
            prompt.push_str(&truncate_chars(&chat_message.content, 700));
            prompt.push('\n');
        }
        prompt.push('\n');
    }

    prompt.push_str("Transcript excerpts:\n");
    for segment in segments {
        prompt.push_str(&format!(
            "[{}] {}\n",
            format_timestamp(segment.start_s),
            truncate_chars(&segment.text, 900)
        ));
    }

    prompt.push_str("\nLearner question:\n");
    prompt.push_str(message);
    prompt.push_str("\n\nAnswer:");
    prompt
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

fn format_timestamp(value: f64) -> String {
    let total_seconds = value.max(0.0).floor() as u64;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}
