use axum::{
    extract::{Path, Query, State},
    Json,
};
use nexalearn_backend::{
    assessment::handler::{
        get_course_random_questions, get_video_questions, QuestionFilters, RandomQuestionFilters,
    },
    chat::{
        delete_user_chat, get_user_chat, list_user_chats, send_chat_message, start_video_chat,
        ChatListFilters,
    },
    config::Config,
    courses::{create_course, delete_course, list_courses},
    db,
    document_processing::{
        infer_document_recovery_stage, prepare_document_recovery, DocumentProcessStage,
    },
    documents::{delete_document, get_document, list_documents, DocumentFilters},
    embedding::OllamaEmbeddingClient,
    ingestion::worker::{infer_video_recovery_stage, prepare_video_recovery, VideoProcessStage},
    llm::gemma::GemmaClient,
    models::{CreateCourseRequest, StartTranscriptChatRequest, TranscriptChatRequest},
    storage::rustfs::RustFsClient,
    videos::{get_video, get_video_transcript, list_videos},
    whisper::client::WhisperClient,
    AppError, AppState,
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::{sleep, Duration},
};
use uuid::Uuid;

struct TestContext {
    state: AppState,
}

impl TestContext {
    async fn new() -> Option<Self> {
        Self::new_with_gemma("http://127.0.0.1:8100").await
    }

    async fn new_with_gemma(gemma_base_url: &str) -> Option<Self> {
        let database_url = match std::env::var("TEST_DATABASE_URL") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => {
                eprintln!("skipping backend integration test; TEST_DATABASE_URL is not set");
                return None;
            }
        };

        std::env::set_var("AWS_EC2_METADATA_DISABLED", "true");

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("connect to TEST_DATABASE_URL");
        db::run_migrations(&pool)
            .await
            .expect("run database migrations");

        let config = Config {
            database_url,
            rustfs_endpoint: "http://127.0.0.1:9000".to_string(),
            rustfs_bucket: "nexalearn-test".to_string(),
            rustfs_access_key: "minio".to_string(),
            rustfs_secret_key: "minio12345".to_string(),
            gemma_base_url: gemma_base_url.to_string(),
            gemma_model: "test-model".to_string(),
            gemma_max_concurrent_requests: 1,
            gemma_request_timeout_seconds: 5,
            ollama_base_url: "http://127.0.0.1:11434".to_string(),
            embedding_model: "nomic-embed-text".to_string(),
            semantic_cache_threshold: 0.70,
            whisper_url: "http://127.0.0.1:8000".to_string(),
            tmp_dir: "/tmp/nexalearn-tests".to_string(),
            bind_addr: "127.0.0.1:0".to_string(),
        };

        let storage = RustFsClient::new(&config)
            .await
            .expect("construct storage client");
        let gemma = GemmaClient::new(
            &config.gemma_base_url,
            &config.gemma_model,
            config.gemma_max_concurrent_requests,
            config.gemma_request_timeout_seconds,
        );
        let embeddings =
            OllamaEmbeddingClient::new(&config.ollama_base_url, &config.embedding_model);
        let whisper = WhisperClient::new(&config.whisper_url);
        let state = AppState::new(config, pool, storage, gemma, embeddings, whisper);

        Some(Self { state })
    }
}

#[tokio::test]
async fn courses_can_be_created_listed_blocked_and_deleted() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };

    let title = format!("Integration Course {}", Uuid::new_v4());
    let created = create_course(
        State(ctx.state.clone()),
        Json(CreateCourseRequest {
            title: format!("  {title}  "),
            description: Some("  description  ".to_string()),
        }),
    )
    .await
    .expect("create course")
    .0;

    assert_eq!(created.title, title);
    assert_eq!(created.description.as_deref(), Some("description"));
    assert_eq!(created.video_count, 0);

    let duplicate = create_course(
        State(ctx.state.clone()),
        Json(CreateCourseRequest {
            title: title.clone(),
            description: None,
        }),
    )
    .await
    .expect_err("duplicate title should conflict");
    assert!(matches!(duplicate, AppError::Conflict(_)));

    let listed = list_courses(State(ctx.state.clone()))
        .await
        .expect("list courses")
        .0;
    assert!(listed.courses.iter().any(|course| course.id == created.id));

    let video_id = insert_video(&ctx.state.pool, created.id, "Blocking lesson").await;
    let blocked = delete_course(State(ctx.state.clone()), Path(created.id))
        .await
        .expect_err("course with video cannot be deleted");
    assert!(matches!(blocked, AppError::Conflict(_)));

    sqlx::query("DELETE FROM videos WHERE id = $1")
        .bind(video_id)
        .execute(&ctx.state.pool)
        .await
        .expect("delete blocking video");

    let deleted = delete_course(State(ctx.state.clone()), Path(created.id))
        .await
        .expect("delete empty course")
        .0;
    assert!(deleted.deleted);
}

#[tokio::test]
async fn course_validation_and_document_delete_conflicts_are_enforced() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };

    let blank = create_course(
        State(ctx.state.clone()),
        Json(CreateCourseRequest {
            title: "   ".to_string(),
            description: None,
        }),
    )
    .await
    .expect_err("blank title should be rejected");
    assert!(matches!(blank, AppError::BadRequest(_)));

    let course_id = insert_course(&ctx.state.pool, "Document blocked course").await;
    let document_id = insert_document(&ctx.state.pool, course_id, "Blocking PDF").await;

    let blocked = delete_course(State(ctx.state.clone()), Path(course_id))
        .await
        .expect_err("course with document cannot be deleted");
    assert!(matches!(blocked, AppError::Conflict(_)));

    sqlx::query("DELETE FROM documents WHERE id = $1")
        .bind(document_id)
        .execute(&ctx.state.pool)
        .await
        .expect("delete blocking document");

    let deleted = delete_course(State(ctx.state.clone()), Path(course_id))
        .await
        .expect("delete course after document removal")
        .0;
    assert!(deleted.deleted);

    let missing = delete_course(State(ctx.state.clone()), Path(Uuid::new_v4()))
        .await
        .expect_err("missing course should be not found");
    assert!(matches!(missing, AppError::NotFound(_)));
}

#[tokio::test]
async fn videos_transcripts_and_questions_can_be_read() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };

    let course_id = insert_course(&ctx.state.pool, "Video integration").await;
    let video_id = insert_video(&ctx.state.pool, course_id, "Transcript lesson").await;
    let topic_id = insert_topic(&ctx.state.pool, video_id, "Intro").await;
    let transcript_id = insert_transcript(&ctx.state.pool, video_id).await;
    insert_transcript_segment(
        &ctx.state.pool,
        transcript_id,
        0,
        0.0,
        2.0,
        "Hello transcript",
    )
    .await;
    let question_id =
        insert_question(&ctx.state.pool, video_id, Some(topic_id), "What is said?").await;
    insert_choice(&ctx.state.pool, question_id, "A", true).await;
    insert_choice(&ctx.state.pool, question_id, "B", false).await;

    let videos = list_videos(State(ctx.state.clone()))
        .await
        .expect("list videos")
        .0;
    assert!(videos.videos.iter().any(|video| video.id == video_id));

    let detail = get_video(State(ctx.state.clone()), Path(video_id))
        .await
        .expect("get video detail")
        .0;
    assert_eq!(detail.video.id, video_id);
    assert_eq!(detail.video.topic_count, 1);
    assert_eq!(detail.video.question_count, 1);
    assert_eq!(detail.topics.len(), 1);

    let transcript = get_video_transcript(State(ctx.state.clone()), Path(video_id))
        .await
        .expect("get transcript")
        .0;
    assert_eq!(transcript.full_text.as_deref(), Some("Hello transcript"));
    assert_eq!(transcript.segments.len(), 1);

    let questions = get_video_questions(
        State(ctx.state.clone()),
        Path(video_id),
        Query(QuestionFilters {
            topic_id: None,
            r#type: None,
        }),
    )
    .await
    .expect("get video questions")
    .0;
    assert_eq!(questions.topics.len(), 1);
    assert_eq!(questions.topics[0].questions[0].choices.len(), 2);

    let random = get_course_random_questions(
        State(ctx.state.clone()),
        Path(course_id),
        Query(RandomQuestionFilters {
            count: Some(1),
            r#type: Some("mcq".to_string()),
        }),
    )
    .await
    .expect("get course random question")
    .0;
    assert_eq!(random.questions.len(), 1);
    assert_eq!(random.questions[0].id, question_id);
}

#[tokio::test]
async fn video_question_filters_empty_transcripts_and_missing_records_are_handled() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };

    let course_id = insert_course(&ctx.state.pool, "Video edge integration").await;
    let video_id = insert_video(&ctx.state.pool, course_id, "Filter lesson").await;
    let topic_one_id = insert_topic(&ctx.state.pool, video_id, "One").await;
    let topic_two_id = insert_topic_with_order(&ctx.state.pool, video_id, "Two", 1).await;
    let question_one_id =
        insert_question(&ctx.state.pool, video_id, Some(topic_one_id), "One?").await;
    let question_two_id = insert_typed_question(
        &ctx.state.pool,
        video_id,
        Some(topic_two_id),
        "Two?",
        "true_false",
    )
    .await;
    insert_choice(&ctx.state.pool, question_one_id, "A", true).await;
    insert_choice(&ctx.state.pool, question_two_id, "A", true).await;

    let empty_transcript = get_video_transcript(State(ctx.state.clone()), Path(video_id))
        .await
        .expect("video without transcript returns empty transcript")
        .0;
    assert_eq!(empty_transcript.full_text, None);
    assert!(empty_transcript.segments.is_empty());

    let topic_filtered = get_video_questions(
        State(ctx.state.clone()),
        Path(video_id),
        Query(QuestionFilters {
            topic_id: Some(topic_one_id),
            r#type: None,
        }),
    )
    .await
    .expect("filter questions by topic")
    .0;
    assert_eq!(topic_filtered.topics.len(), 1);
    assert_eq!(topic_filtered.topics[0].questions.len(), 1);
    assert_eq!(topic_filtered.topics[0].questions[0].id, question_one_id);

    let type_filtered = get_video_questions(
        State(ctx.state.clone()),
        Path(video_id),
        Query(QuestionFilters {
            topic_id: None,
            r#type: Some("true_false".to_string()),
        }),
    )
    .await
    .expect("filter questions by type")
    .0;
    assert_eq!(type_filtered.topics[0].questions.len(), 0);
    assert_eq!(type_filtered.topics[1].questions[0].id, question_two_id);

    let invalid_random_count = get_course_random_questions(
        State(ctx.state.clone()),
        Path(course_id),
        Query(RandomQuestionFilters {
            count: Some(101),
            r#type: None,
        }),
    )
    .await
    .expect_err("count above max should fail");
    assert!(matches!(invalid_random_count, AppError::BadRequest(_)));

    let missing_video = get_video(State(ctx.state.clone()), Path(Uuid::new_v4()))
        .await
        .expect_err("missing video should be not found");
    assert!(matches!(missing_video, AppError::NotFound(_)));
}

#[tokio::test]
async fn document_metadata_can_be_listed_read_and_deleted() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };

    let course_id = insert_course(&ctx.state.pool, "Document integration").await;
    let document_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO documents
            (id, course_id, title, file_name, rustfs_key, content_type, status, page_count)
        VALUES ($1, $2, 'PDF Guide', 'guide.pdf', 'documents/test/guide.pdf', 'application/pdf', 'ready', 3)
        "#,
    )
    .bind(document_id)
    .bind(course_id)
    .execute(&ctx.state.pool)
    .await
    .expect("insert document");

    let listed = list_documents(
        State(ctx.state.clone()),
        Query(DocumentFilters {
            course_id: Some(course_id),
        }),
    )
    .await
    .expect("list documents")
    .0;
    assert!(listed
        .documents
        .iter()
        .any(|document| document.id == document_id));

    let document = get_document(State(ctx.state.clone()), Path(document_id))
        .await
        .expect("get document")
        .0;
    assert_eq!(document.title, "PDF Guide");
    assert_eq!(document.page_count, Some(3));

    let deleted = delete_document(State(ctx.state.clone()), Path(document_id))
        .await
        .expect("delete document")
        .0;
    assert!(deleted.deleted);

    let remaining: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM documents WHERE id = $1")
        .bind(document_id)
        .fetch_one(&ctx.state.pool)
        .await
        .expect("count documents");
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn document_delete_removes_chunks_and_invalidates_course_semantic_cache() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };

    let course_id = insert_course(&ctx.state.pool, "Document cascade integration").await;
    let video_id = insert_video(&ctx.state.pool, course_id, "Cached lesson").await;
    let document_id = insert_document(&ctx.state.pool, course_id, "Cached PDF").await;
    insert_document_chunk(&ctx.state.pool, document_id).await;
    insert_semantic_cache_entry(&ctx.state.pool, video_id).await;

    let listed = list_documents(
        State(ctx.state.clone()),
        Query(DocumentFilters { course_id: None }),
    )
    .await
    .expect("list all documents")
    .0;
    assert!(listed.documents.iter().any(|item| item.id == document_id));

    let _ = delete_document(State(ctx.state.clone()), Path(document_id))
        .await
        .expect("delete document with chunks and cache");

    let chunk_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM document_chunks WHERE document_id = $1")
            .bind(document_id)
            .fetch_one(&ctx.state.pool)
            .await
            .expect("count deleted chunks");
    assert_eq!(chunk_count, 0);

    let cache_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM semantic_chat_cache WHERE video_id = $1")
            .bind(video_id)
            .fetch_one(&ctx.state.pool)
            .await
            .expect("count invalidated cache");
    assert_eq!(cache_count, 0);

    let missing = get_document(State(ctx.state.clone()), Path(document_id))
        .await
        .expect_err("deleted document should be missing");
    assert!(matches!(missing, AppError::NotFound(_)));
}

#[tokio::test]
async fn recovery_prepares_video_and_document_from_resume_stage() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };

    let course_id = insert_course(&ctx.state.pool, "Recovery integration").await;
    let video_id = insert_video(&ctx.state.pool, course_id, "Recoverable lesson").await;
    let transcript_id = insert_transcript(&ctx.state.pool, video_id).await;
    insert_transcript_segment(
        &ctx.state.pool,
        transcript_id,
        0,
        0.0,
        2.0,
        "Recovery transcript",
    )
    .await;
    let topic_id = insert_topic(&ctx.state.pool, video_id, "Recovery topic").await;
    let question_id =
        insert_question(&ctx.state.pool, video_id, Some(topic_id), "Old question?").await;
    insert_choice(&ctx.state.pool, question_id, "A", true).await;
    sqlx::query("INSERT INTO summaries (video_id, content) VALUES ($1, 'Old summary')")
        .bind(video_id)
        .execute(&ctx.state.pool)
        .await
        .expect("insert summary");
    insert_semantic_cache_entry(&ctx.state.pool, video_id).await;
    sqlx::query("UPDATE videos SET status = 'failed', error_msg = 'boom' WHERE id = $1")
        .bind(video_id)
        .execute(&ctx.state.pool)
        .await
        .expect("mark video failed");

    let inferred = infer_video_recovery_stage(&ctx.state, video_id, "failed")
        .await
        .expect("infer video recovery stage");
    assert_eq!(inferred, VideoProcessStage::Summarizing);

    prepare_video_recovery(&ctx.state, video_id, VideoProcessStage::GeneratingQuestions)
        .await
        .expect("prepare video recovery");

    let status: String = sqlx::query_scalar("SELECT status FROM videos WHERE id = $1")
        .bind(video_id)
        .fetch_one(&ctx.state.pool)
        .await
        .expect("load video status");
    assert_eq!(status, "generating_questions");
    assert_count(&ctx.state.pool, "transcripts", "video_id", video_id, 1).await;
    assert_count(&ctx.state.pool, "topics", "video_id", video_id, 1).await;
    assert_count(&ctx.state.pool, "questions", "video_id", video_id, 0).await;
    assert_count(&ctx.state.pool, "summaries", "video_id", video_id, 0).await;
    assert_count(
        &ctx.state.pool,
        "semantic_chat_cache",
        "video_id",
        video_id,
        0,
    )
    .await;

    let document_id = insert_document(&ctx.state.pool, course_id, "Recoverable PDF").await;
    sqlx::query("UPDATE documents SET status = 'failed', error_msg = 'embed failed', full_text = 'Saved extracted text', page_count = 4 WHERE id = $1")
        .bind(document_id)
        .execute(&ctx.state.pool)
        .await
        .expect("mark document failed with text");
    insert_document_chunk(&ctx.state.pool, document_id).await;
    insert_semantic_cache_entry(&ctx.state.pool, video_id).await;

    let document_stage = infer_document_recovery_stage(&ctx.state, document_id, "failed")
        .await
        .expect("infer document recovery stage");
    assert_eq!(document_stage, DocumentProcessStage::Embedding);

    prepare_document_recovery(&ctx.state, document_id, DocumentProcessStage::Embedding)
        .await
        .expect("prepare document recovery");
    let (document_status, full_text): (String, Option<String>) =
        sqlx::query_as("SELECT status, full_text FROM documents WHERE id = $1")
            .bind(document_id)
            .fetch_one(&ctx.state.pool)
            .await
            .expect("load document");
    assert_eq!(document_status, "embedding");
    assert_eq!(full_text.as_deref(), Some("Saved extracted text"));
    assert_count(
        &ctx.state.pool,
        "document_chunks",
        "document_id",
        document_id,
        0,
    )
    .await;
    assert_count(
        &ctx.state.pool,
        "semantic_chat_cache",
        "video_id",
        video_id,
        0,
    )
    .await;
}

#[tokio::test]
async fn chat_history_can_be_started_listed_read_and_deleted() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };

    let user_id = Uuid::new_v4();
    let course_id = insert_course(&ctx.state.pool, "Chat integration").await;
    let video_id = insert_video(&ctx.state.pool, course_id, "Chat lesson").await;

    let chat = start_video_chat(
        State(ctx.state.clone()),
        Path(video_id),
        Json(StartTranscriptChatRequest {
            user_id,
            name: Some("Exam prep".to_string()),
        }),
    )
    .await
    .expect("start chat")
    .0;
    assert_eq!(chat.name, "Exam prep");
    assert_eq!(chat.messages.len(), 0);

    let listed = list_user_chats(
        State(ctx.state.clone()),
        Path(user_id),
        Query(ChatListFilters {
            video_id: Some(video_id),
        }),
    )
    .await
    .expect("list chats")
    .0;
    assert_eq!(listed.chats.len(), 1);
    assert_eq!(listed.chats[0].conversation_id, chat.conversation_id);

    insert_chat_message(&ctx.state.pool, chat.conversation_id, "user", "Hello?").await;
    insert_chat_message(&ctx.state.pool, chat.conversation_id, "assistant", "Hi.").await;

    let fetched = get_user_chat(
        State(ctx.state.clone()),
        Path((user_id, chat.conversation_id)),
    )
    .await
    .expect("get chat")
    .0;
    assert_eq!(fetched.name, "Exam prep");
    assert_eq!(fetched.messages.len(), 2);

    let wrong_user = Uuid::new_v4();
    let wrong_user_read = get_user_chat(
        State(ctx.state.clone()),
        Path((wrong_user, chat.conversation_id)),
    )
    .await
    .expect_err("wrong user cannot read chat");
    assert!(matches!(wrong_user_read, AppError::NotFound(_)));

    let deleted = delete_user_chat(
        State(ctx.state.clone()),
        Path((user_id, chat.conversation_id)),
    )
    .await
    .expect("delete chat")
    .0;
    assert!(deleted.deleted);

    let message_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM chat_messages WHERE conversation_id = $1")
            .bind(chat.conversation_id)
            .fetch_one(&ctx.state.pool)
            .await
            .expect("count deleted chat messages");
    assert_eq!(message_count, 0);
}

#[tokio::test]
async fn chat_validation_and_filters_are_enforced() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };

    let user_id = Uuid::new_v4();
    let course_id = insert_course(&ctx.state.pool, "Chat filter integration").await;
    let video_one_id = insert_video(&ctx.state.pool, course_id, "First chat lesson").await;
    let video_two_id = insert_video(&ctx.state.pool, course_id, "Second chat lesson").await;

    let blank_name = start_video_chat(
        State(ctx.state.clone()),
        Path(video_one_id),
        Json(StartTranscriptChatRequest {
            user_id,
            name: Some("   ".to_string()),
        }),
    )
    .await
    .expect_err("blank chat name should be rejected");
    assert!(matches!(blank_name, AppError::BadRequest(_)));

    let chat_one = start_video_chat(
        State(ctx.state.clone()),
        Path(video_one_id),
        Json(StartTranscriptChatRequest {
            user_id,
            name: Some("One".to_string()),
        }),
    )
    .await
    .expect("start first chat")
    .0;
    let chat_two = start_video_chat(
        State(ctx.state.clone()),
        Path(video_two_id),
        Json(StartTranscriptChatRequest {
            user_id,
            name: Some("Two".to_string()),
        }),
    )
    .await
    .expect("start second chat")
    .0;

    let all = list_user_chats(
        State(ctx.state.clone()),
        Path(user_id),
        Query(ChatListFilters { video_id: None }),
    )
    .await
    .expect("list all chats")
    .0;
    assert_eq!(all.chats.len(), 2);

    let filtered = list_user_chats(
        State(ctx.state.clone()),
        Path(user_id),
        Query(ChatListFilters {
            video_id: Some(video_two_id),
        }),
    )
    .await
    .expect("list filtered chats")
    .0;
    assert_eq!(filtered.chats.len(), 1);
    assert_eq!(filtered.chats[0].conversation_id, chat_two.conversation_id);

    let wrong_delete = delete_user_chat(
        State(ctx.state.clone()),
        Path((Uuid::new_v4(), chat_one.conversation_id)),
    )
    .await
    .expect_err("wrong user cannot delete chat");
    assert!(matches!(wrong_delete, AppError::NotFound(_)));
}

#[tokio::test]
async fn chat_send_persists_user_message_and_background_assistant_response() {
    let gemma_base_url = start_mock_server(json_response(
        r#"{"choices":[{"message":{"content":"Grounded assistant answer"}}]}"#,
    ))
    .await;
    let Some(ctx) = TestContext::new_with_gemma(&gemma_base_url).await else {
        return;
    };
    let user_id = Uuid::new_v4();
    let course_id = insert_course(&ctx.state.pool, "Chat send integration").await;
    let video_id = insert_video(&ctx.state.pool, course_id, "Chat send lesson").await;
    sqlx::query("INSERT INTO summaries (video_id, content) VALUES ($1, 'Short summary')")
        .bind(video_id)
        .execute(&ctx.state.pool)
        .await
        .expect("insert summary");
    let transcript_id = insert_transcript(&ctx.state.pool, video_id).await;
    insert_transcript_segment(
        &ctx.state.pool,
        transcript_id,
        0,
        0.0,
        4.0,
        "Spark actions are operations that trigger execution.",
    )
    .await;
    let chat = start_video_chat(
        State(ctx.state.clone()),
        Path(video_id),
        Json(StartTranscriptChatRequest {
            user_id,
            name: Some("Send flow".to_string()),
        }),
    )
    .await
    .expect("start chat")
    .0;

    let response = send_chat_message(
        State(ctx.state.clone()),
        Path(chat.conversation_id),
        Json(TranscriptChatRequest {
            user_id,
            message: "What are Spark actions?".to_string(),
            history: Vec::new(),
        }),
    )
    .await
    .expect("send chat message")
    .0;

    assert!(response.is_waiting);
    assert!(response.answer.is_none());

    let final_chat =
        wait_for_chat_ready(&ctx.state.pool, &ctx.state, user_id, chat.conversation_id).await;
    assert!(!final_chat.is_waiting);
    assert_eq!(final_chat.messages.len(), 2);
    assert_eq!(final_chat.messages[0].role, "user");
    assert_eq!(final_chat.messages[1].role, "assistant");
    assert_eq!(final_chat.messages[1].content, "Grounded assistant answer");
}

#[tokio::test]
async fn chat_send_validates_input_and_waiting_state() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };
    let user_id = Uuid::new_v4();
    let course_id = insert_course(&ctx.state.pool, "Chat validation integration").await;
    let video_id = insert_video(&ctx.state.pool, course_id, "Chat validation lesson").await;
    let chat = start_video_chat(
        State(ctx.state.clone()),
        Path(video_id),
        Json(StartTranscriptChatRequest {
            user_id,
            name: Some("Validation".to_string()),
        }),
    )
    .await
    .expect("start chat")
    .0;

    let blank = send_chat_message(
        State(ctx.state.clone()),
        Path(chat.conversation_id),
        Json(TranscriptChatRequest {
            user_id,
            message: "   ".to_string(),
            history: Vec::new(),
        }),
    )
    .await
    .expect_err("blank message should fail");
    assert!(matches!(blank, AppError::BadRequest(_)));

    let too_long = send_chat_message(
        State(ctx.state.clone()),
        Path(chat.conversation_id),
        Json(TranscriptChatRequest {
            user_id,
            message: "x".repeat(2_001),
            history: Vec::new(),
        }),
    )
    .await
    .expect_err("long message should fail");
    assert!(matches!(too_long, AppError::BadRequest(_)));

    sqlx::query("UPDATE chat_conversations SET is_waiting = true WHERE id = $1")
        .bind(chat.conversation_id)
        .execute(&ctx.state.pool)
        .await
        .expect("mark chat waiting");
    let waiting = send_chat_message(
        State(ctx.state.clone()),
        Path(chat.conversation_id),
        Json(TranscriptChatRequest {
            user_id,
            message: "Hello".to_string(),
            history: Vec::new(),
        }),
    )
    .await
    .expect_err("waiting chat should conflict");
    assert!(matches!(waiting, AppError::Conflict(_)));
}

async fn insert_course(pool: &PgPool, label: &str) -> Uuid {
    let course_id = Uuid::new_v4();
    sqlx::query("INSERT INTO courses (id, title, description) VALUES ($1, $2, $3)")
        .bind(course_id)
        .bind(format!("{label} {course_id}"))
        .bind("integration test")
        .execute(pool)
        .await
        .expect("insert course");
    course_id
}

async fn insert_video(pool: &PgPool, course_id: Uuid, title: &str) -> Uuid {
    let video_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO videos (id, course_id, title, rustfs_key, status) VALUES ($1, $2, $3, $4, 'ready')",
    )
    .bind(video_id)
    .bind(course_id)
    .bind(title)
    .bind(format!("videos/{video_id}/original.mp4"))
    .execute(pool)
    .await
    .expect("insert video");
    video_id
}

async fn insert_topic(pool: &PgPool, video_id: Uuid, label: &str) -> Uuid {
    insert_topic_with_order(pool, video_id, label, 0).await
}

async fn insert_topic_with_order(
    pool: &PgPool,
    video_id: Uuid,
    label: &str,
    seq_order: i32,
) -> Uuid {
    let topic_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO topics (id, video_id, label, start_s, end_s, seq_order) VALUES ($1, $2, $3, 0, 10, $4)",
    )
    .bind(topic_id)
    .bind(video_id)
    .bind(label)
    .bind(seq_order)
    .execute(pool)
    .await
    .expect("insert topic");
    topic_id
}

async fn insert_transcript(pool: &PgPool, video_id: Uuid) -> Uuid {
    let transcript_id = Uuid::new_v4();
    sqlx::query("INSERT INTO transcripts (id, video_id, full_text, language) VALUES ($1, $2, 'Hello transcript', 'en')")
        .bind(transcript_id)
        .bind(video_id)
        .execute(pool)
        .await
        .expect("insert transcript");
    transcript_id
}

async fn insert_transcript_segment(
    pool: &PgPool,
    transcript_id: Uuid,
    seq_index: i32,
    start_s: f64,
    end_s: f64,
    text: &str,
) {
    sqlx::query(
        "INSERT INTO transcript_segments (transcript_id, seq_index, start_s, end_s, text) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(transcript_id)
    .bind(seq_index)
    .bind(start_s)
    .bind(end_s)
    .bind(text)
    .execute(pool)
    .await
    .expect("insert transcript segment");
}

async fn insert_question(
    pool: &PgPool,
    video_id: Uuid,
    topic_id: Option<Uuid>,
    stem: &str,
) -> Uuid {
    insert_typed_question(pool, video_id, topic_id, stem, "mcq").await
}

async fn insert_typed_question(
    pool: &PgPool,
    video_id: Uuid,
    topic_id: Option<Uuid>,
    stem: &str,
    question_type: &str,
) -> Uuid {
    let question_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO questions (id, video_id, topic_id, stem, question_type, difficulty) VALUES ($1, $2, $3, $4, $5, 'easy')",
    )
    .bind(question_id)
    .bind(video_id)
    .bind(topic_id)
    .bind(stem)
    .bind(question_type)
    .execute(pool)
    .await
    .expect("insert question");
    question_id
}

async fn insert_choice(pool: &PgPool, question_id: Uuid, label: &str, is_correct: bool) {
    sqlx::query(
        "INSERT INTO choices (question_id, label, text, is_correct) VALUES ($1, $2, $3, $4)",
    )
    .bind(question_id)
    .bind(label)
    .bind(format!("Choice {label}"))
    .bind(is_correct)
    .execute(pool)
    .await
    .expect("insert choice");
}

async fn insert_document(pool: &PgPool, course_id: Uuid, title: &str) -> Uuid {
    let document_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO documents
            (id, course_id, title, file_name, rustfs_key, content_type, status, page_count)
        VALUES ($1, $2, $3, 'guide.pdf', $4, 'application/pdf', 'ready', 3)
        "#,
    )
    .bind(document_id)
    .bind(course_id)
    .bind(title)
    .bind(format!("documents/{document_id}/guide.pdf"))
    .execute(pool)
    .await
    .expect("insert document");
    document_id
}

async fn insert_document_chunk(pool: &PgPool, document_id: Uuid) {
    sqlx::query(
        r#"
        INSERT INTO document_chunks
            (document_id, seq_index, page_start, page_end, content, embedding_model, embedding)
        VALUES ($1, 0, 1, 1, 'cached document text', 'nomic-embed-text', $2::vector)
        "#,
    )
    .bind(document_id)
    .bind(zero_vector_literal())
    .execute(pool)
    .await
    .expect("insert document chunk");
}

async fn insert_semantic_cache_entry(pool: &PgPool, video_id: Uuid) {
    sqlx::query(
        r#"
        INSERT INTO semantic_chat_cache
            (video_id, embedding_model, question, embedding, answer, sources_json)
        VALUES ($1, 'nomic-embed-text', 'cached?', $2::vector, 'cached answer', '[]')
        "#,
    )
    .bind(video_id)
    .bind(zero_vector_literal())
    .execute(pool)
    .await
    .expect("insert semantic cache entry");
}

async fn insert_chat_message(pool: &PgPool, conversation_id: Uuid, role: &str, content: &str) {
    sqlx::query("INSERT INTO chat_messages (conversation_id, role, content) VALUES ($1, $2, $3)")
        .bind(conversation_id)
        .bind(role)
        .bind(content)
        .execute(pool)
        .await
        .expect("insert chat message");
}

async fn assert_count(pool: &PgPool, table: &str, column: &str, id: Uuid, expected: i64) {
    let sql = format!("SELECT COUNT(*)::BIGINT FROM {table} WHERE {column} = $1");
    let count: i64 = sqlx::query_scalar(&sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("count rows");
    assert_eq!(count, expected, "unexpected count for {table}.{column}");
}

fn zero_vector_literal() -> String {
    format!("[{}]", vec!["0"; 768].join(","))
}

async fn wait_for_chat_ready(
    pool: &PgPool,
    state: &AppState,
    user_id: Uuid,
    conversation_id: Uuid,
) -> nexalearn_backend::models::TranscriptChatHistoryResponse {
    for _ in 0..40 {
        let waiting: bool =
            sqlx::query_scalar("SELECT is_waiting FROM chat_conversations WHERE id = $1")
                .bind(conversation_id)
                .fetch_one(pool)
                .await
                .expect("load waiting state");
        if !waiting {
            return get_user_chat(State(state.clone()), Path((user_id, conversation_id)))
                .await
                .expect("load final chat")
                .0;
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("chat {conversation_id} did not finish");
}

async fn start_mock_server(response: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 8192];
        let _ = stream.read(&mut buffer).await.unwrap();
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    format!("http://{addr}")
}

fn json_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    )
}
