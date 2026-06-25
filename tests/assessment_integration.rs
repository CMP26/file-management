use axum::{
    extract::{Path, Query, State},
    Json,
};
use nexalearn_backend::{
    assessment::{
        context::transcript_context_for_question,
        handler::{
            delete_user_exam_attempt, get_attempt_status, list_user_exam_attempts,
            start_exam_attempt, submit_attempt, UserExamFilters,
        },
        justifier::response_for_answer,
    },
    config::Config,
    db,
    embedding::OllamaEmbeddingClient,
    llm::gemma::GemmaClient,
    models::{StartExamRequest, SubmitAnswerInput, SubmitAttemptRequest},
    storage::rustfs::RustFsClient,
    whisper::client::WhisperClient,
    AppError, AppState,
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::sleep,
};
use uuid::Uuid;

struct TestContext {
    state: AppState,
}

struct SeededAssessment {
    user_id: Uuid,
    video_one_id: Uuid,
    video_two_id: Uuid,
    question_one_id: Uuid,
    question_two_id: Uuid,
    other_course_question_id: Uuid,
}

impl TestContext {
    async fn new() -> Option<Self> {
        Self::new_with_gemma("http://127.0.0.1:8100").await
    }

    async fn new_with_gemma(gemma_base_url: &str) -> Option<Self> {
        let database_url = match std::env::var("TEST_DATABASE_URL") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => {
                eprintln!("skipping assessment integration test; TEST_DATABASE_URL is not set");
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
async fn question_bank_attempt_accepts_same_course_questions_and_can_be_deleted() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };
    let seeded = seed_assessment(&ctx.state.pool).await;

    let started = start_exam_attempt(
        State(ctx.state.clone()),
        Path(seeded.video_one_id),
        Json(StartExamRequest {
            user_id: seeded.user_id,
        }),
    )
    .await
    .expect("start attempt")
    .0;

    let submitted = submit_attempt(
        State(ctx.state.clone()),
        Path(started.attempt_id),
        Json(SubmitAttemptRequest {
            answers: vec![
                SubmitAnswerInput {
                    question_id: seeded.question_one_id,
                    user_answer: "A".to_string(),
                },
                SubmitAnswerInput {
                    question_id: seeded.question_two_id,
                    user_answer: "B".to_string(),
                },
            ],
        }),
    )
    .await
    .expect("submit same-course question bank attempt")
    .0;

    assert!(submitted.is_waiting);
    assert_eq!(submitted.pending_count, 2);

    wait_for_graded_answers(&ctx.state.pool, started.attempt_id, 2).await;

    let status = get_attempt_status(State(ctx.state.clone()), Path(started.attempt_id))
        .await
        .expect("get graded attempt")
        .0;
    assert_eq!(status.status, "graded");
    assert!(!status.is_waiting);
    assert_eq!(status.answers.len(), 2);
    assert_eq!(status.total_score, 200);

    let listed = list_user_exam_attempts(
        State(ctx.state.clone()),
        Path(seeded.user_id),
        Query(UserExamFilters { video_id: None }),
    )
    .await
    .expect("list user attempts")
    .0;
    assert!(listed
        .attempts
        .iter()
        .any(|attempt| attempt.attempt_id == started.attempt_id));

    let deleted = delete_user_exam_attempt(
        State(ctx.state.clone()),
        Path((seeded.user_id, started.attempt_id)),
    )
    .await
    .expect("delete user attempt")
    .0;
    assert!(deleted.deleted);

    let answer_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM attempt_answers WHERE attempt_id = $1")
            .bind(started.attempt_id)
            .fetch_one(&ctx.state.pool)
            .await
            .expect("count deleted answers");
    assert_eq!(answer_count, 0);
}

#[tokio::test]
async fn question_bank_attempt_rejects_questions_from_other_courses() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };
    let seeded = seed_assessment(&ctx.state.pool).await;

    let started = start_exam_attempt(
        State(ctx.state.clone()),
        Path(seeded.video_one_id),
        Json(StartExamRequest {
            user_id: seeded.user_id,
        }),
    )
    .await
    .expect("start attempt")
    .0;

    let error = submit_attempt(
        State(ctx.state.clone()),
        Path(started.attempt_id),
        Json(SubmitAttemptRequest {
            answers: vec![SubmitAnswerInput {
                question_id: seeded.other_course_question_id,
                user_answer: "A".to_string(),
            }],
        }),
    )
    .await
    .expect_err("reject cross-course question");

    assert!(matches!(error, AppError::BadRequest(_)));

    let answer_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM attempt_answers WHERE attempt_id = $1")
            .bind(started.attempt_id)
            .fetch_one(&ctx.state.pool)
            .await
            .expect("count rejected answers");
    assert_eq!(answer_count, 0);
}

#[tokio::test]
async fn attempts_can_be_filtered_and_reject_duplicate_submission() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };
    let seeded = seed_assessment(&ctx.state.pool).await;

    let first = start_exam_attempt(
        State(ctx.state.clone()),
        Path(seeded.video_one_id),
        Json(StartExamRequest {
            user_id: seeded.user_id,
        }),
    )
    .await
    .expect("start first attempt")
    .0;
    let second = start_exam_attempt(
        State(ctx.state.clone()),
        Path(seeded.video_two_id),
        Json(StartExamRequest {
            user_id: seeded.user_id,
        }),
    )
    .await
    .expect("start second attempt")
    .0;

    let all = list_user_exam_attempts(
        State(ctx.state.clone()),
        Path(seeded.user_id),
        Query(UserExamFilters { video_id: None }),
    )
    .await
    .expect("list all attempts")
    .0;
    assert!(all
        .attempts
        .iter()
        .any(|attempt| attempt.attempt_id == first.attempt_id));
    assert!(all
        .attempts
        .iter()
        .any(|attempt| attempt.attempt_id == second.attempt_id));

    let filtered = list_user_exam_attempts(
        State(ctx.state.clone()),
        Path(seeded.user_id),
        Query(UserExamFilters {
            video_id: Some(seeded.video_two_id),
        }),
    )
    .await
    .expect("list filtered attempts")
    .0;
    assert_eq!(filtered.attempts.len(), 1);
    assert_eq!(filtered.attempts[0].attempt_id, second.attempt_id);

    let _ = submit_attempt(
        State(ctx.state.clone()),
        Path(first.attempt_id),
        Json(SubmitAttemptRequest {
            answers: vec![SubmitAnswerInput {
                question_id: seeded.question_one_id,
                user_answer: "A".to_string(),
            }],
        }),
    )
    .await
    .expect("submit first attempt");

    let duplicate = submit_attempt(
        State(ctx.state.clone()),
        Path(first.attempt_id),
        Json(SubmitAttemptRequest {
            answers: vec![SubmitAnswerInput {
                question_id: seeded.question_one_id,
                user_answer: "A".to_string(),
            }],
        }),
    )
    .await
    .expect_err("duplicate submission should conflict");
    assert!(matches!(duplicate, AppError::Conflict(_)));
}

#[tokio::test]
async fn attempt_delete_is_user_scoped_and_missing_attempts_are_not_found() {
    let Some(ctx) = TestContext::new().await else {
        return;
    };
    let seeded = seed_assessment(&ctx.state.pool).await;

    let started = start_exam_attempt(
        State(ctx.state.clone()),
        Path(seeded.video_one_id),
        Json(StartExamRequest {
            user_id: seeded.user_id,
        }),
    )
    .await
    .expect("start attempt")
    .0;

    let wrong_user_delete = delete_user_exam_attempt(
        State(ctx.state.clone()),
        Path((Uuid::new_v4(), started.attempt_id)),
    )
    .await
    .expect_err("wrong user cannot delete attempt");
    assert!(matches!(wrong_user_delete, AppError::NotFound(_)));

    let missing_status = get_attempt_status(State(ctx.state.clone()), Path(Uuid::new_v4()))
        .await
        .expect_err("missing attempt status should fail");
    assert!(matches!(missing_status, AppError::Database(_)));

    let deleted = delete_user_exam_attempt(
        State(ctx.state.clone()),
        Path((seeded.user_id, started.attempt_id)),
    )
    .await
    .expect("owner can delete attempt")
    .0;
    assert!(deleted.deleted);
}

#[tokio::test]
async fn transcript_context_and_answer_justification_use_topic_context_and_cache_result() {
    let gemma_base_url = start_mock_server(json_response(
        r#"{"choices":[{"message":{"content":"You understood the basics, but review the exact term."}}]}"#,
    ))
    .await;
    let Some(ctx) = TestContext::new_with_gemma(&gemma_base_url).await else {
        return;
    };
    let seeded = seed_assessment(&ctx.state.pool).await;
    let topic_id = insert_topic(&ctx.state.pool, seeded.video_one_id, 10.0, 20.0).await;
    let transcript_id = insert_transcript(&ctx.state.pool, seeded.video_one_id).await;
    insert_transcript_segment(&ctx.state.pool, transcript_id, 0, 0.0, 5.0, "outside topic").await;
    insert_transcript_segment(
        &ctx.state.pool,
        transcript_id,
        1,
        12.0,
        18.0,
        "inside topic context",
    )
    .await;
    sqlx::query("UPDATE questions SET topic_id = $1 WHERE id = $2")
        .bind(topic_id)
        .bind(seeded.question_one_id)
        .execute(&ctx.state.pool)
        .await
        .expect("attach topic to question");

    let question = sqlx::query_as("SELECT * FROM questions WHERE id = $1")
        .bind(seeded.question_one_id)
        .fetch_one(&ctx.state.pool)
        .await
        .expect("load question");
    let context = transcript_context_for_question(&ctx.state, &question)
        .await
        .expect("topic context");
    assert!(context.contains("inside topic context"));
    assert!(!context.contains("outside topic"));

    let started = start_exam_attempt(
        State(ctx.state.clone()),
        Path(seeded.video_one_id),
        Json(StartExamRequest {
            user_id: seeded.user_id,
        }),
    )
    .await
    .expect("start attempt")
    .0;
    let _ = submit_attempt(
        State(ctx.state.clone()),
        Path(started.attempt_id),
        Json(SubmitAttemptRequest {
            answers: vec![SubmitAnswerInput {
                question_id: seeded.question_one_id,
                user_answer: "B".to_string(),
            }],
        }),
    )
    .await
    .expect("submit attempt");
    wait_for_graded_answers(&ctx.state.pool, started.attempt_id, 1).await;
    let answer_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM attempt_answers WHERE attempt_id = $1 AND question_id = $2",
    )
    .bind(started.attempt_id)
    .bind(seeded.question_one_id)
    .fetch_one(&ctx.state.pool)
    .await
    .expect("load answer id");

    let justification = response_for_answer(&ctx.state, started.attempt_id, answer_id)
        .await
        .expect("generate justification");
    assert_eq!(
        justification.justification,
        "You understood the basics, but review the exact term."
    );

    let cached = response_for_answer(&ctx.state, started.attempt_id, answer_id)
        .await
        .expect("reuse cached justification");
    assert_eq!(cached.justification, justification.justification);
}

async fn seed_assessment(pool: &PgPool) -> SeededAssessment {
    let user_id = Uuid::new_v4();
    let course_one_id = Uuid::new_v4();
    let course_two_id = Uuid::new_v4();
    let video_one_id = Uuid::new_v4();
    let video_two_id = Uuid::new_v4();
    let video_other_id = Uuid::new_v4();
    let question_one_id = Uuid::new_v4();
    let question_two_id = Uuid::new_v4();
    let other_course_question_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO courses (id, title, description)
        VALUES ($1, $2, 'test course'), ($3, $4, 'other test course')
        "#,
    )
    .bind(course_one_id)
    .bind(format!("Integration course {course_one_id}"))
    .bind(course_two_id)
    .bind(format!("Integration course {course_two_id}"))
    .execute(pool)
    .await
    .expect("insert courses");

    sqlx::query(
        r#"
        INSERT INTO videos (id, course_id, title, rustfs_key, status)
        VALUES
            ($1, $2, 'Lesson one', 'test/one.mp4', 'ready'),
            ($3, $2, 'Lesson two', 'test/two.mp4', 'ready'),
            ($4, $5, 'Other lesson', 'test/other.mp4', 'ready')
        "#,
    )
    .bind(video_one_id)
    .bind(course_one_id)
    .bind(video_two_id)
    .bind(video_other_id)
    .bind(course_two_id)
    .execute(pool)
    .await
    .expect("insert videos");

    sqlx::query(
        r#"
        INSERT INTO questions (id, video_id, stem, question_type, difficulty)
        VALUES
            ($1, $2, 'Pick A', 'mcq', 'easy'),
            ($3, $4, 'Pick B', 'mcq', 'easy'),
            ($5, $6, 'Other course question', 'mcq', 'easy')
        "#,
    )
    .bind(question_one_id)
    .bind(video_one_id)
    .bind(question_two_id)
    .bind(video_two_id)
    .bind(other_course_question_id)
    .bind(video_other_id)
    .execute(pool)
    .await
    .expect("insert questions");

    insert_choices(pool, question_one_id, "A").await;
    insert_choices(pool, question_two_id, "B").await;
    insert_choices(pool, other_course_question_id, "A").await;

    SeededAssessment {
        user_id,
        video_one_id,
        video_two_id,
        question_one_id,
        question_two_id,
        other_course_question_id,
    }
}

async fn insert_topic(pool: &PgPool, video_id: Uuid, start_s: f64, end_s: f64) -> Uuid {
    let topic_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO topics (id, video_id, label, start_s, end_s, seq_order) VALUES ($1, $2, 'Topic', $3, $4, 0)",
    )
    .bind(topic_id)
    .bind(video_id)
    .bind(start_s)
    .bind(end_s)
    .execute(pool)
    .await
    .expect("insert topic");
    topic_id
}

async fn insert_transcript(pool: &PgPool, video_id: Uuid) -> Uuid {
    let transcript_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transcripts (id, video_id, full_text, language) VALUES ($1, $2, 'full transcript fallback', 'en')",
    )
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

async fn insert_choices(pool: &PgPool, question_id: Uuid, correct_label: &str) {
    for (label, is_correct) in [("A", correct_label == "A"), ("B", correct_label == "B")] {
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
}

async fn wait_for_graded_answers(pool: &PgPool, attempt_id: Uuid, expected_count: i64) {
    for _ in 0..40 {
        let graded_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM attempt_answers WHERE attempt_id = $1 AND graded_at IS NOT NULL",
        )
        .bind(attempt_id)
        .fetch_one(pool)
        .await
        .expect("count graded answers");

        if graded_count == expected_count {
            return;
        }

        sleep(Duration::from_millis(50)).await;
    }

    panic!("attempt {attempt_id} did not finish grading {expected_count} answers");
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
