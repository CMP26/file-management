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
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::{collections::HashSet, convert::Infallible};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

#[path = "chat/prompt.rs"]
mod prompt;
#[path = "chat/semantic_cache.rs"]
mod semantic_cache;

#[derive(Debug, Clone)]
struct TranscriptSegment {
    seq_index: i32,
    start_s: f64,
    end_s: f64,
    text: String,
}

#[derive(Debug, Clone)]
struct DocumentContext {
    document_id: Uuid,
    document_title: String,
    seq_index: i32,
    page_start: i32,
    page_end: i32,
    text: String,
}

#[derive(Debug)]
struct ChatMessageRow {
    id: Uuid,
    role: String,
    content: String,
    sources_json: Option<String>,
    cached: bool,
    cache_similarity: Option<f32>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ConversationContext {
    user_id: Uuid,
    video_id: Uuid,
    course_id: Uuid,
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
        (status = 409, description = "Chat is already waiting for an LLM response"),
        (status = 502, description = "LLM service error")
    )
)]
pub async fn send_chat_message(
    State(state): State<AppState>,
    Path(conversation_id): Path<Uuid>,
    Json(payload): Json<TranscriptChatRequest>,
) -> AppResult<Json<TranscriptChatResponse>> {
    clear_stale_waiting_chats(&state).await?;

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
    if conversation.is_waiting || !claim_conversation_waiting(&state, conversation_id).await? {
        return Err(AppError::conflict(
            "chat is already waiting for an LLM response",
        ));
    }

    let stored_history = match load_chat_messages(&state, conversation_id).await {
        Ok(history) => history,
        Err(error) => {
            release_conversation_waiting(&state, conversation_id).await;
            return Err(error);
        }
    };
    let prompt_history = if stored_history.is_empty() {
        payload.history.clone()
    } else {
        stored_history
            .iter()
            .map(|message| TranscriptChatMessage {
                role: message.role.clone(),
                content: message.content.clone(),
            })
            .collect()
    };

    let user_message_id =
        match insert_chat_message(&state, conversation_id, "user", message, &[], false, None).await
        {
            Ok(message_id) => message_id,
            Err(error) => {
                release_conversation_waiting(&state, conversation_id).await;
                return Err(error);
            }
        };

    let question_embedding = match semantic_cache::lookup(&state, conversation.video_id, message)
        .await
    {
        Ok((_embedding, Some(hit))) => {
            let assistant_message_id = match insert_chat_message(
                &state,
                conversation_id,
                "assistant",
                &hit.answer,
                &hit.sources,
                true,
                Some(hit.similarity),
            )
            .await
            {
                Ok(message_id) => message_id,
                Err(error) => {
                    release_conversation_waiting(&state, conversation_id).await;
                    return Err(error);
                }
            };
            if let Err(error) = semantic_cache::record_hit(&state, hit.id).await {
                tracing::warn!(cache_id = %hit.id, error = %error, "failed to record semantic cache hit");
            }
            set_conversation_waiting(&state, conversation_id, false).await?;
            let _ = state.chat_events.send(conversation_id);
            tracing::info!(
                conversation_id = %conversation_id,
                video_id = %conversation.video_id,
                similarity = hit.similarity,
                "semantic chat cache hit"
            );
            return Ok(Json(TranscriptChatResponse {
                conversation_id,
                video_id: conversation.video_id,
                name: conversation.name,
                is_waiting: false,
                user_message_id,
                assistant_message_id: Some(assistant_message_id),
                answer: Some(hit.answer),
                sources: hit.sources,
                cached: true,
                cache_similarity: Some(hit.similarity),
            }));
        }
        Ok((embedding, None)) => Some(embedding),
        Err(error) => {
            tracing::warn!(conversation_id = %conversation_id, error = %error, "semantic cache unavailable; falling back to Gemma");
            None
        }
    };

    let worker_state = state.clone();
    let worker_conversation = conversation.clone();
    let worker_message = message.to_string();
    let worker_history = prompt_history.clone();
    let worker_embedding = question_embedding;
    tokio::spawn(async move {
        if let Err(error) = complete_chat_message(
            worker_state,
            conversation_id,
            worker_conversation,
            worker_message,
            worker_history,
            worker_embedding,
        )
        .await
        {
            tracing::error!(conversation_id = %conversation_id, error = %error, "chat llm worker failed");
        }
    });

    Ok(Json(TranscriptChatResponse {
        conversation_id,
        video_id: conversation.video_id,
        name: conversation.name,
        is_waiting: true,
        user_message_id,
        assistant_message_id: None,
        answer: None,
        sources: Vec::new(),
        cached: false,
        cache_similarity: None,
    }))
}

async fn complete_chat_message(
    state: AppState,
    conversation_id: Uuid,
    conversation: ConversationContext,
    message: String,
    prompt_history: Vec<TranscriptChatMessage>,
    question_embedding: Option<Vec<f32>>,
) -> AppResult<()> {
    tracing::info!(conversation_id = %conversation_id, "chat llm worker started");

    let result = generate_chat_answer(&state, &conversation, &message, &prompt_history).await;
    match result {
        Ok((answer, sources)) => {
            match insert_chat_message(
                &state,
                conversation_id,
                "assistant",
                &answer,
                &sources,
                false,
                None,
            )
            .await
            {
                Ok(_) => {
                    tracing::info!(conversation_id = %conversation_id, "chat llm worker saved assistant response");
                    if let Some(embedding) = question_embedding {
                        if let Err(error) = semantic_cache::store(
                            &state,
                            conversation.video_id,
                            &message,
                            &embedding,
                            &answer,
                            &sources,
                        )
                        .await
                        {
                            tracing::warn!(conversation_id = %conversation_id, error = %error, "failed to store semantic chat cache entry");
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(conversation_id = %conversation_id, error = %error, "chat llm worker failed to save assistant response");
                }
            }
        }
        Err(error) => {
            let fallback = format!(
                "I could not get a response from the LLM for this message. Please try again. Error: {error}"
            );
            if let Err(insert_error) = insert_chat_message(
                &state,
                conversation_id,
                "assistant",
                &fallback,
                &[],
                false,
                None,
            )
            .await
            {
                tracing::error!(conversation_id = %conversation_id, error = %insert_error, "chat llm worker failed to save fallback response");
            }
            tracing::warn!(conversation_id = %conversation_id, error = %error, "chat llm worker saved fallback response");
        }
    }

    set_conversation_waiting(&state, conversation_id, false).await?;
    tracing::info!(conversation_id = %conversation_id, "chat llm worker completed");
    Ok(())
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
    let selected_documents = match retrieve_document_context(
        state,
        conversation.course_id,
        message,
        prompt_history,
        8,
    )
    .await
    {
        Ok(documents) => documents,
        Err(error) => {
            tracing::warn!(error = %error, "document retrieval unavailable; continuing with transcript context");
            Vec::new()
        }
    };
    let prompt = prompt::build_transcript_chat_prompt(
        &conversation.video_title,
        summary.as_deref(),
        message,
        prompt_history,
        &selected_segments,
        &selected_documents,
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
            source_type: "transcript".to_string(),
            seq_index: segment.seq_index,
            start_s: segment.start_s,
            end_s: segment.end_s,
            text: segment.text,
            document_id: None,
            document_title: None,
            page_start: None,
            page_end: None,
        })
        .chain(
            selected_documents
                .into_iter()
                .map(|document| TranscriptChatSource {
                    source_type: "document".to_string(),
                    seq_index: document.seq_index,
                    start_s: 0.0,
                    end_s: 0.0,
                    text: document.text,
                    document_id: Some(document.document_id),
                    document_title: Some(document.document_title),
                    page_start: Some(document.page_start),
                    page_end: Some(document.page_end),
                }),
        )
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
    clear_stale_waiting_chats(&state).await?;

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
    clear_stale_waiting_chats(&state).await?;

    Ok(Json(
        chat_history_response(&state, user_id, conversation_id).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/users/{user_id}/chats/{conversation_id}/events",
    tag = "Chat",
    params(
        ("user_id" = Uuid, Path, description = "User id"),
        ("conversation_id" = Uuid, Path, description = "Chat conversation id")
    ),
    responses(
        (status = 200, description = "Server-sent chat updates while an assistant response is pending"),
        (status = 404, description = "Chat not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn stream_user_chat_events(
    State(state): State<AppState>,
    Path((user_id, conversation_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Sse<ReceiverStream<Result<Event, Infallible>>>> {
    clear_stale_waiting_chats(&state).await?;
    let mut chat_events = state.chat_events.subscribe();
    let initial = chat_history_response(&state, user_id, conversation_id).await?;

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);
    let worker_state = state.clone();

    tokio::spawn(async move {
        let _ = send_chat_event(&tx, &initial).await;
        if !initial.is_waiting {
            return;
        }

        loop {
            match chat_events.recv().await {
                Ok(changed_conversation_id) if changed_conversation_id == conversation_id => {
                    match chat_history_response(&worker_state, user_id, conversation_id).await {
                        Ok(snapshot) => {
                            let is_waiting = snapshot.is_waiting;
                            if send_chat_event(&tx, &snapshot).await.is_err() {
                                return;
                            }
                            if !is_waiting {
                                return;
                            }
                        }
                        Err(error) => {
                            let event = Event::default().event("error").data(error.to_string());
                            let _ = tx.send(Ok(event)).await;
                            return;
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    let event = Event::default().event("error").data(error.to_string());
                    let _ = tx.send(Ok(event)).await;
                    return;
                }
            }
        }
    });

    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
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

async fn chat_history_response(
    state: &AppState,
    user_id: Uuid,
    conversation_id: Uuid,
) -> AppResult<TranscriptChatHistoryResponse> {
    let conversation = conversation_context(state, user_id, conversation_id).await?;
    let messages = load_chat_messages(state, conversation_id)
        .await?
        .into_iter()
        .map(message_response_from_row)
        .collect();

    Ok(TranscriptChatHistoryResponse {
        user_id: conversation.user_id,
        video_id: conversation.video_id,
        video_title: conversation.video_title,
        conversation_id,
        name: conversation.name,
        is_waiting: conversation.is_waiting,
        messages,
    })
}

async fn send_chat_event(
    tx: &mpsc::Sender<Result<Event, Infallible>>,
    snapshot: &TranscriptChatHistoryResponse,
) -> Result<(), mpsc::error::SendError<Result<Event, Infallible>>> {
    let data = serde_json::to_string(snapshot).unwrap_or_else(|error| {
        format!(
            "{{\"error\":\"failed to serialize chat snapshot\",\"message\":\"{}\"}}",
            error
        )
    });
    tx.send(Ok(Event::default().event("chat").data(data))).await
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
    sqlx::query_as::<_, (Uuid, Uuid, Uuid, Uuid, String, bool, String)>(
        r#"
        SELECT c.id, c.user_id, c.video_id, v.course_id, c.name, c.is_waiting, v.title
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
        |(_, user_id, video_id, course_id, name, is_waiting, video_title)| ConversationContext {
            user_id,
            video_id,
            course_id,
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
    let _ = state.chat_events.send(conversation_id);
    tracing::info!(conversation_id = %conversation_id, is_waiting, "chat waiting state updated");
    Ok(())
}

async fn claim_conversation_waiting(state: &AppState, conversation_id: Uuid) -> AppResult<bool> {
    let result = sqlx::query(
        r#"
        UPDATE chat_conversations
        SET is_waiting = true, updated_at = now()
        WHERE id = $1 AND is_waiting = false
        "#,
    )
    .bind(conversation_id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 1 {
        let _ = state.chat_events.send(conversation_id);
        Ok(true)
    } else {
        Ok(false)
    }
}

async fn release_conversation_waiting(state: &AppState, conversation_id: Uuid) {
    if let Err(error) = set_conversation_waiting(state, conversation_id, false).await {
        tracing::error!(conversation_id = %conversation_id, error = %error, "failed to release chat waiting state");
    }
}

async fn clear_stale_waiting_chats(state: &AppState) -> AppResult<()> {
    let stale_after_seconds = (state.config.gemma_request_timeout_seconds + 30).max(90) as f64;
    let stale_conversation_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"
        UPDATE chat_conversations
        SET is_waiting = false, updated_at = now()
        WHERE is_waiting = true
          AND updated_at < now() - ($1 * interval '1 second')
        RETURNING id
        "#,
    )
    .bind(stale_after_seconds)
    .fetch_all(&state.pool)
    .await?;
    for conversation_id in stale_conversation_ids {
        let _ = state.chat_events.send(conversation_id);
        tracing::info!(conversation_id = %conversation_id, "stale chat waiting state cleared");
    }
    Ok(())
}

async fn insert_chat_message(
    state: &AppState,
    conversation_id: Uuid,
    role: &str,
    content: &str,
    sources: &[TranscriptChatSource],
    cached: bool,
    cache_similarity: Option<f32>,
) -> AppResult<Uuid> {
    let sources_json = if sources.is_empty() {
        None
    } else {
        Some(serde_json::to_string(sources)?)
    };

    let message_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO chat_messages
            (conversation_id, role, content, sources_json, cached, cache_similarity)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(conversation_id)
    .bind(role)
    .bind(content)
    .bind(sources_json)
    .bind(cached)
    .bind(cache_similarity)
    .fetch_one(&state.pool)
    .await?;

    Ok(message_id)
}

async fn load_chat_messages(
    state: &AppState,
    conversation_id: Uuid,
) -> AppResult<Vec<ChatMessageRow>> {
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            bool,
            Option<f32>,
            DateTime<Utc>,
        ),
    >(
        r#"
        SELECT id, role, content, sources_json, cached, cache_similarity, created_at
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
        |(id, role, content, sources_json, cached, cache_similarity, created_at)| ChatMessageRow {
            id,
            role,
            content,
            sources_json,
            cached,
            cache_similarity,
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
        cached: row.cached,
        cache_similarity: row.cache_similarity,
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

async fn retrieve_document_context(
    state: &AppState,
    course_id: Uuid,
    message: &str,
    history: &[TranscriptChatMessage],
    limit: i64,
) -> AppResult<Vec<DocumentContext>> {
    let mut query = message.to_string();
    for item in history.iter().rev().take(4) {
        query.push(' ');
        query.push_str(&item.content);
    }
    let embedding = state.embeddings.embed_query(&query).await?;
    let vector = embedding_vector_literal(&embedding)?;
    let rows = sqlx::query_as::<_, (Uuid, String, i32, i32, i32, String, f32)>(
        r#"
        SELECT d.id, d.title, dc.seq_index, dc.page_start, dc.page_end, dc.content,
               (1 - (dc.embedding <=> $2::vector))::REAL AS similarity
        FROM document_chunks dc
        JOIN documents d ON d.id = dc.document_id
        WHERE d.course_id = $1 AND d.status = 'ready' AND dc.embedding_model = $3
        ORDER BY dc.embedding <=> $2::vector
        LIMIT $4
        "#,
    )
    .bind(course_id)
    .bind(vector)
    .bind(state.embeddings.model())
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter(|row| row.6 >= 0.35)
        .map(|row| DocumentContext {
            document_id: row.0,
            document_title: row.1,
            seq_index: row.2,
            page_start: row.3,
            page_end: row.4,
            text: row.5,
        })
        .collect())
}

fn embedding_vector_literal(embedding: &[f32]) -> AppResult<String> {
    if embedding.len() != 768 || embedding.iter().any(|value| !value.is_finite()) {
        return Err(AppError::external("invalid document query embedding"));
    }
    Ok(format!(
        "[{}]",
        embedding
            .iter()
            .map(f32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        chat_name, embedding_vector_literal, message_response_from_row, select_relevant_segments,
        tokenize, ChatMessageRow, TranscriptSegment,
    };
    use crate::{models::TranscriptChatMessage, AppError};
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn chat_name_defaults_trims_and_validates() {
        assert_eq!(chat_name(None).unwrap(), "Untitled chat");
        assert_eq!(chat_name(Some("  Exam prep  ")).unwrap(), "Exam prep");
        assert!(matches!(
            chat_name(Some("   ")),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            chat_name(Some(&"x".repeat(121))),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn tokenize_keeps_meaningful_ascii_terms_only() {
        let tokens = tokenize("A Java-based Spark job, in JVM!");

        assert!(tokens.contains("java"));
        assert!(tokens.contains("based"));
        assert!(tokens.contains("spark"));
        assert!(tokens.contains("jvm"));
        assert!(!tokens.contains("a"));
        assert!(!tokens.contains("in"));
    }

    #[test]
    fn select_relevant_segments_scores_message_and_recent_history() {
        let segments = vec![
            segment(0, "Java virtual machine overview"),
            segment(1, "Spark transformations and actions"),
            segment(2, "Hadoop storage layer"),
        ];
        let history = vec![TranscriptChatMessage {
            role: "user".to_string(),
            content: "Tell me about Spark".to_string(),
        }];

        let selected = select_relevant_segments("What are actions?", &history, &segments, 2);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].seq_index, 1);
    }

    #[test]
    fn select_relevant_segments_falls_back_to_initial_segments() {
        let segments = vec![
            segment(0, "alpha beta"),
            segment(1, "gamma delta"),
            segment(2, "epsilon zeta"),
        ];

        let selected = select_relevant_segments("unmatched", &[], &segments, 2);

        assert_eq!(
            selected
                .into_iter()
                .map(|segment| segment.seq_index)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn message_response_parses_sources_and_cache_metadata() {
        let message_id = Uuid::new_v4();
        let created_at = Utc::now();
        let row = ChatMessageRow {
            id: message_id,
            role: "assistant".to_string(),
            content: "Answer".to_string(),
            sources_json: Some(
                r#"[{"source_type":"transcript","seq_index":3,"start_s":1.0,"end_s":2.0,"text":"source"}]"#
                    .to_string(),
            ),
            cached: true,
            cache_similarity: Some(0.82),
            created_at,
        };

        let response = message_response_from_row(row);

        assert_eq!(response.id, message_id);
        assert!(response.cached);
        assert_eq!(response.cache_similarity, Some(0.82));
        assert_eq!(response.sources.len(), 1);
        assert_eq!(response.sources[0].seq_index, 3);
    }

    #[test]
    fn message_response_ignores_invalid_sources_json() {
        let row = ChatMessageRow {
            id: Uuid::new_v4(),
            role: "assistant".to_string(),
            content: "Answer".to_string(),
            sources_json: Some("not json".to_string()),
            cached: false,
            cache_similarity: None,
            created_at: Utc::now(),
        };

        assert!(message_response_from_row(row).sources.is_empty());
    }

    #[test]
    fn embedding_vector_literal_validates_dimension_and_finiteness() {
        let embedding = vec![0.25; 768];
        let literal = embedding_vector_literal(&embedding).unwrap();

        assert!(literal.starts_with("[0.25,0.25"));
        assert!(matches!(
            embedding_vector_literal(&[0.1, 0.2]),
            Err(AppError::External(_))
        ));

        let mut invalid = vec![0.0; 768];
        invalid[10] = f32::NAN;
        assert!(matches!(
            embedding_vector_literal(&invalid),
            Err(AppError::External(_))
        ));
    }

    fn segment(seq_index: i32, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            seq_index,
            start_s: f64::from(seq_index),
            end_s: f64::from(seq_index + 1),
            text: text.to_string(),
        }
    }
}
