use crate::{
    ingestion::{
        audio::{create_playback_video, extract_audio},
        question_gen::generate_assessment_questions,
        segmenter::chunk_segments,
        summarizer::summarize,
        topic_labeler::label_chunk,
    },
    models::{GeneratedChoice, GeneratedQuestion, TranscriptSegmentInput},
    AppResult, AppState,
};
use std::path::PathBuf;
use uuid::Uuid;

const QUESTIONS_PER_TOPIC: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VideoProcessStage {
    ExtractingAudio,
    LabelingTopics,
    GeneratingQuestions,
    Summarizing,
}

impl VideoProcessStage {
    pub fn as_status(self) -> &'static str {
        match self {
            Self::ExtractingAudio => "extracting_audio",
            Self::LabelingTopics => "labeling_topics",
            Self::GeneratingQuestions => "generating_questions",
            Self::Summarizing => "summarizing",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" | "extracting_audio" | "transcribing" => Some(Self::ExtractingAudio),
            "labeling_topics" => Some(Self::LabelingTopics),
            "generating_questions" => Some(Self::GeneratingQuestions),
            "summarizing" => Some(Self::Summarizing),
            _ => None,
        }
    }
}

pub async fn process_video(state: AppState, video_id: Uuid) -> AppResult<()> {
    process_video_from_stage(state, video_id, VideoProcessStage::ExtractingAudio).await
}

pub async fn process_video_from_stage(
    state: AppState,
    video_id: Uuid,
    stage: VideoProcessStage,
) -> AppResult<()> {
    match process_video_inner(state.clone(), video_id, stage).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let message = error.to_string();
            let _ = update_status(&state, video_id, "failed", Some(&message)).await;
            Err(error)
        }
    }
}

pub async fn prepare_video_recovery(
    state: &AppState,
    video_id: Uuid,
    stage: VideoProcessStage,
) -> AppResult<()> {
    let mut transaction = state.pool.begin().await?;
    match stage {
        VideoProcessStage::ExtractingAudio => {
            sqlx::query("DELETE FROM transcripts WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM topics WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM questions WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM summaries WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
        }
        VideoProcessStage::LabelingTopics => {
            sqlx::query("DELETE FROM questions WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM topics WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM summaries WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
        }
        VideoProcessStage::GeneratingQuestions => {
            sqlx::query("DELETE FROM questions WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM summaries WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
        }
        VideoProcessStage::Summarizing => {
            sqlx::query("DELETE FROM summaries WHERE video_id = $1")
                .bind(video_id)
                .execute(&mut *transaction)
                .await?;
        }
    }
    sqlx::query("DELETE FROM semantic_chat_cache WHERE video_id = $1")
        .bind(video_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE videos SET status = $1, error_msg = NULL WHERE id = $2")
        .bind(stage.as_status())
        .bind(video_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    let _ = state.video_events.send(video_id);
    Ok(())
}

pub async fn infer_video_recovery_stage(
    state: &AppState,
    video_id: Uuid,
    status: &str,
) -> AppResult<VideoProcessStage> {
    if let Some(stage) = VideoProcessStage::parse(status) {
        if stage == VideoProcessStage::ExtractingAudio {
            return Ok(stage);
        }
    }

    let transcript_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM transcripts WHERE video_id = $1")
            .bind(video_id)
            .fetch_one(&state.pool)
            .await?;
    if transcript_count == 0 {
        return Ok(VideoProcessStage::ExtractingAudio);
    }

    let topic_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM topics WHERE video_id = $1")
            .bind(video_id)
            .fetch_one(&state.pool)
            .await?;
    if topic_count == 0 {
        return Ok(VideoProcessStage::LabelingTopics);
    }

    let question_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM questions WHERE video_id = $1")
            .bind(video_id)
            .fetch_one(&state.pool)
            .await?;
    if question_count == 0 {
        return Ok(VideoProcessStage::GeneratingQuestions);
    }

    Ok(VideoProcessStage::Summarizing)
}

async fn process_video_inner(
    state: AppState,
    video_id: Uuid,
    start_stage: VideoProcessStage,
) -> AppResult<()> {
    let tmp_dir = PathBuf::from(&state.config.tmp_dir);
    tokio::fs::create_dir_all(&tmp_dir).await?;

    let original_key: String = sqlx::query_scalar("SELECT rustfs_key FROM videos WHERE id = $1")
        .bind(video_id)
        .fetch_one(&state.pool)
        .await?;

    let video_path = tmp_dir.join(format!("{video_id}.mp4"));
    let playback_path = tmp_dir.join(format!("{video_id}.playback.mp4"));
    let wav_path = tmp_dir.join(format!("{video_id}.wav"));

    let transcript = if start_stage <= VideoProcessStage::ExtractingAudio {
        update_status(&state, video_id, "extracting_audio", None).await?;
        let video_bytes = state.storage.download(&original_key).await?;
        tokio::fs::write(&video_path, video_bytes).await?;
        match create_playback_video(&video_path, &playback_path).await {
            Ok(()) => {
                let playback_bytes = tokio::fs::read(&playback_path).await?;
                state
                    .storage
                    .upload(
                        &format!("videos/{video_id}/playback.mp4"),
                        playback_bytes,
                        "video/mp4",
                    )
                    .await?;
            }
            Err(error) => {
                tracing::warn!(video_id = %video_id, error = %error, "failed to create browser playback video");
            }
        }
        extract_audio(&video_path, &wav_path).await?;

        update_status(&state, video_id, "transcribing", None).await?;
        let transcript = state.whisper.transcribe(&wav_path, "en").await?;
        insert_transcript(&state, video_id, &transcript).await?;
        upload_transcript_artifacts(&state, video_id, &transcript).await?;
        transcript
    } else {
        load_transcript(&state, video_id).await?
    };

    let topic_records = if start_stage <= VideoProcessStage::LabelingTopics {
        update_status(&state, video_id, "labeling_topics", None).await?;
        label_topics(&state, video_id, &transcript.segments).await?
    } else {
        load_topic_records(&state, video_id, &transcript.segments).await?
    };

    if start_stage <= VideoProcessStage::GeneratingQuestions {
        update_status(&state, video_id, "generating_questions", None).await?;
        generate_and_insert_questions(&state, video_id, &topic_records).await?;
    }

    if start_stage <= VideoProcessStage::Summarizing {
        update_status(&state, video_id, "summarizing", None).await?;
        let summary = summarize(&state.gemma, &transcript.full_text)
            .await
            .unwrap_or_else(|_| "Summary unavailable in mock mode.".to_string());

        sqlx::query(
            r#"
            INSERT INTO summaries (video_id, content)
            VALUES ($1, $2)
            "#,
        )
        .bind(video_id)
        .bind(&summary)
        .execute(&state.pool)
        .await?;
    }

    update_status(&state, video_id, "ready", None).await?;

    let _ = tokio::fs::remove_file(video_path).await;
    let _ = tokio::fs::remove_file(playback_path).await;
    let _ = tokio::fs::remove_file(wav_path).await;

    Ok(())
}

async fn insert_transcript(
    state: &AppState,
    video_id: Uuid,
    transcript: &crate::models::TranscribeResponse,
) -> AppResult<()> {
    let transcript_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO transcripts (video_id, full_text, language)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(video_id)
    .bind(&transcript.full_text)
    .bind("en")
    .fetch_one(&state.pool)
    .await?;

    for (seq_index, segment) in transcript.segments.iter().enumerate() {
        sqlx::query(
            r#"
            INSERT INTO transcript_segments (transcript_id, seq_index, start_s, end_s, text)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(transcript_id)
        .bind(seq_index as i32)
        .bind(segment.start)
        .bind(segment.end)
        .bind(&segment.text)
        .execute(&state.pool)
        .await?;
    }

    Ok(())
}

async fn upload_transcript_artifacts(
    state: &AppState,
    video_id: Uuid,
    transcript: &crate::models::TranscribeResponse,
) -> AppResult<()> {
    state
        .storage
        .upload(
            &format!("videos/{video_id}/transcript.txt"),
            transcript.full_text.as_bytes().to_vec(),
            "text/plain; charset=utf-8",
        )
        .await?;

    let vtt_text = transcript_to_vtt(&transcript.segments);
    state
        .storage
        .upload(
            &format!("videos/{video_id}/transcript.vtt"),
            vtt_text.into_bytes(),
            "text/vtt; charset=utf-8",
        )
        .await?;
    Ok(())
}

async fn load_transcript(
    state: &AppState,
    video_id: Uuid,
) -> AppResult<crate::models::TranscribeResponse> {
    let (transcript_id, full_text): (Uuid, String) =
        sqlx::query_as("SELECT id, full_text FROM transcripts WHERE video_id = $1 ORDER BY created_at DESC LIMIT 1")
            .bind(video_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| crate::AppError::external("cannot resume video processing without a saved transcript"))?;
    let segments = sqlx::query_as::<_, (f64, f64, String)>(
        r#"
        SELECT start_s, end_s, text
        FROM transcript_segments
        WHERE transcript_id = $1
        ORDER BY seq_index ASC
        "#,
    )
    .bind(transcript_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(start, end, text)| TranscriptSegmentInput { start, end, text })
    .collect::<Vec<_>>();

    Ok(crate::models::TranscribeResponse {
        full_text,
        segments,
    })
}

async fn label_topics(
    state: &AppState,
    video_id: Uuid,
    segments: &[TranscriptSegmentInput],
) -> AppResult<Vec<(Uuid, String, crate::models::Chunk)>> {
    let chunks = chunk_segments(segments, 300);
    let mut topic_records = Vec::new();

    for chunk in &chunks {
        let label = label_chunk(&state.gemma, &chunk.text).await?;

        let topic_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO topics (video_id, label, start_s, end_s, seq_order)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id
            "#,
        )
        .bind(video_id)
        .bind(&label)
        .bind(chunk.start_s)
        .bind(chunk.end_s)
        .bind(chunk.seq_index)
        .fetch_one(&state.pool)
        .await?;

        topic_records.push((topic_id, label, chunk.clone()));
    }

    if topic_records.is_empty() {
        return Err(crate::AppError::external(
            "no transcript chunks were available for topic/question generation",
        ));
    }
    Ok(topic_records)
}

async fn load_topic_records(
    state: &AppState,
    video_id: Uuid,
    transcript_segments: &[TranscriptSegmentInput],
) -> AppResult<Vec<(Uuid, String, crate::models::Chunk)>> {
    let records = sqlx::query_as::<_, (Uuid, String, f64, f64, i32)>(
        r#"
        SELECT id, label, start_s, end_s, seq_order
        FROM topics
        WHERE video_id = $1
        ORDER BY seq_order ASC
        "#,
    )
    .bind(video_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(topic_id, label, start_s, end_s, seq_order)| {
        let (text, segments) = topic_transcript_excerpt(transcript_segments, start_s, end_s)
            .unwrap_or_else(|| {
                (
                    label.clone(),
                    Vec::<crate::models::TranscriptSegmentInput>::new(),
                )
            });
        (
            topic_id,
            label.clone(),
            crate::models::Chunk {
                seq_index: seq_order,
                start_s,
                end_s,
                text,
                segments,
            },
        )
    })
    .collect::<Vec<_>>();

    if records.is_empty() {
        return Err(crate::AppError::external(
            "cannot resume question generation without saved topics",
        ));
    }
    Ok(records)
}

fn topic_transcript_excerpt(
    transcript_segments: &[TranscriptSegmentInput],
    start_s: f64,
    end_s: f64,
) -> Option<(String, Vec<TranscriptSegmentInput>)> {
    let segments = transcript_segments
        .iter()
        .filter(|segment| segment.end >= start_s && segment.start <= end_s)
        .cloned()
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return None;
    }

    let text = segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Some((text, segments))
}

async fn generate_and_insert_questions(
    state: &AppState,
    video_id: Uuid,
    topic_records: &[(Uuid, String, crate::models::Chunk)],
) -> AppResult<()> {
    for topic_record in topic_records {
        let (topic_id, label, chunk) = topic_record;
        let existing_count = question_count_for_topic(state, *topic_id).await?;
        if existing_count >= QUESTIONS_PER_TOPIC as i64 {
            tracing::info!(
                video_id = %video_id,
                topic_id = %topic_id,
                topic = %label,
                existing_count,
                "skipping question generation for completed topic"
            );
            continue;
        }
        if existing_count > 0 {
            tracing::warn!(
                video_id = %video_id,
                topic_id = %topic_id,
                topic = %label,
                existing_count,
                "clearing partial generated questions before retry"
            );
            delete_questions_for_topic(state, *topic_id).await?;
        }

        let topic_label = format!(
            "{} ({}s-{}s)",
            label,
            chunk.start_s.round() as i64,
            chunk.end_s.round() as i64
        );
        tracing::info!(
            video_id = %video_id,
            topic = %label,
            chunk_tokens = chunk.text.split_whitespace().count(),
            "generating questions for one topic"
        );
        let generated_questions = generate_assessment_questions(
            &state.gemma,
            &topic_label,
            &chunk.text,
            QUESTIONS_PER_TOPIC,
            QUESTIONS_PER_TOPIC,
        )
        .await?;
        let generated_questions = normalize_generated_questions(
            generated_questions,
            QUESTIONS_PER_TOPIC,
            QUESTIONS_PER_TOPIC,
        )?;
        insert_topic_questions(state, video_id, topic_record, generated_questions).await?;
    }
    Ok(())
}

async fn question_count_for_topic(state: &AppState, topic_id: Uuid) -> AppResult<i64> {
    sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM questions WHERE topic_id = $1")
        .bind(topic_id)
        .fetch_one(&state.pool)
        .await
        .map_err(Into::into)
}

async fn delete_questions_for_topic(state: &AppState, topic_id: Uuid) -> AppResult<()> {
    sqlx::query("DELETE FROM questions WHERE topic_id = $1")
        .bind(topic_id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

async fn update_status(
    state: &AppState,
    video_id: Uuid,
    status: &str,
    error_msg: Option<&str>,
) -> AppResult<()> {
    tracing::info!(video_id = %video_id, status = %status, error_msg = error_msg, "video processing status updated");
    sqlx::query("UPDATE videos SET status = $1, error_msg = $2 WHERE id = $3")
        .bind(status)
        .bind(error_msg)
        .bind(video_id)
        .execute(&state.pool)
        .await?;
    let _ = state.video_events.send(video_id);
    Ok(())
}

fn normalize_generated_questions(
    questions: Vec<GeneratedQuestion>,
    expected_count: usize,
    questions_per_topic: usize,
) -> AppResult<Vec<GeneratedQuestion>> {
    if questions.len() != expected_count {
        return Err(crate::AppError::external(format!(
            "expected exactly {expected_count} assessment questions, got {}",
            questions.len(),
        )));
    }

    Ok(questions
        .into_iter()
        .enumerate()
        .map(|(index, mut question)| {
            let default_type = default_generated_question_type(index, questions_per_topic);
            if question.question_type.trim().is_empty()
                || question.question_type.eq_ignore_ascii_case("essay")
            {
                question.question_type = default_type.to_string();
            }
            question.question_type = normalize_question_type(&question.question_type, default_type);

            if question.difficulty.trim().is_empty() {
                question.difficulty =
                    default_generated_difficulty(index, questions_per_topic).to_string();
            }
            if question
                .rubric
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
            {
                question.rubric =
                    Some(default_generated_rubric(&question.question_type).to_string());
            }
            question.choices =
                normalize_generated_choices(&question.question_type, question.choices);
            question
        })
        .collect())
}

fn normalize_question_type(question_type: &str, fallback: &str) -> String {
    match question_type.trim().to_ascii_lowercase().as_str() {
        "mcq" | "multiple_choice" | "multiple-choice" => "mcq".to_string(),
        "true_false" | "true-false" | "true/false" | "boolean" => "true_false".to_string(),
        "completion" | "fill_blank" | "fill-in-the-blank" | "one_word" | "short_answer" => {
            "completion".to_string()
        }
        "essay" => "essay".to_string(),
        _ => fallback.to_string(),
    }
}

fn normalize_generated_choices(
    question_type: &str,
    choices: Option<Vec<GeneratedChoice>>,
) -> Option<Vec<GeneratedChoice>> {
    match question_type {
        "completion" | "essay" => None,
        "true_false" => Some(normalize_true_false_choices(choices)),
        "mcq" => choices.map(normalize_mcq_choices),
        _ => choices.map(normalize_choice_labels),
    }
}

fn normalize_mcq_choices(choices: Vec<GeneratedChoice>) -> Vec<GeneratedChoice> {
    const LABELS: [&str; 4] = ["A", "B", "C", "D"];

    choices
        .into_iter()
        .take(LABELS.len())
        .enumerate()
        .map(|(index, mut choice)| {
            choice.label = LABELS[index].to_string();
            choice
        })
        .collect()
}

fn normalize_choice_labels(choices: Vec<GeneratedChoice>) -> Vec<GeneratedChoice> {
    choices
        .into_iter()
        .enumerate()
        .map(|(index, mut choice)| {
            if choice.label.trim().chars().count() != 1 {
                choice.label = ((b'A' + index.min(25) as u8) as char).to_string();
            }
            choice
        })
        .collect()
}

fn normalize_true_false_choices(choices: Option<Vec<GeneratedChoice>>) -> Vec<GeneratedChoice> {
    let mut choices = choices.unwrap_or_else(|| {
        vec![
            GeneratedChoice {
                label: "T".to_string(),
                text: "True".to_string(),
                is_correct: true,
            },
            GeneratedChoice {
                label: "F".to_string(),
                text: "False".to_string(),
                is_correct: false,
            },
        ]
    });

    choices.truncate(2);

    for (index, choice) in choices.iter_mut().enumerate() {
        let label = choice.label.trim().to_ascii_lowercase();
        if label == "t" || label == "true" {
            choice.label = "T".to_string();
            if choice.text.trim().is_empty() {
                choice.text = "True".to_string();
            }
        } else if label == "f" || label == "false" {
            choice.label = "F".to_string();
            if choice.text.trim().is_empty() {
                choice.text = "False".to_string();
            }
        } else if choice.text.eq_ignore_ascii_case("true") || index == 0 {
            choice.label = "T".to_string();
            if choice.text.trim().is_empty() {
                choice.text = "True".to_string();
            }
        } else {
            choice.label = "F".to_string();
            if choice.text.trim().is_empty() {
                choice.text = "False".to_string();
            }
        }
    }

    choices
}

fn default_generated_question_type(index: usize, questions_per_topic: usize) -> &'static str {
    match index % questions_per_topic {
        0 => "mcq",
        1 => "true_false",
        _ => "completion",
    }
}

fn default_generated_rubric(question_type: &str) -> &'static str {
    match question_type {
        "completion" => "No expected answer was generated.",
        "mcq" | "true_false" => "Select the single best answer.",
        _ => "Grade for conceptual accuracy in one short sentence.",
    }
}

async fn insert_topic_questions(
    state: &AppState,
    video_id: Uuid,
    topic_record: &(Uuid, String, crate::models::Chunk),
    questions: Vec<GeneratedQuestion>,
) -> AppResult<()> {
    for question in questions {
        let topic_id = topic_record.0;
        let question_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO questions (video_id, topic_id, stem, question_type, difficulty, rubric)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#,
        )
        .bind(video_id)
        .bind(topic_id)
        .bind(&question.stem)
        .bind(&question.question_type)
        .bind(&question.difficulty)
        .bind(&question.rubric)
        .fetch_one(&state.pool)
        .await?;

        if let Some(choices) = question.choices {
            for choice in choices {
                sqlx::query(
                    r#"
                    INSERT INTO choices (question_id, label, text, is_correct)
                    VALUES ($1, $2, $3, $4)
                    "#,
                )
                .bind(question_id)
                .bind(choice.label)
                .bind(choice.text)
                .bind(choice.is_correct)
                .execute(&state.pool)
                .await?;
            }
        }
    }

    Ok(())
}

fn default_generated_difficulty(index: usize, questions_per_topic: usize) -> &'static str {
    match index % questions_per_topic {
        0 => "easy",
        1 => "medium",
        _ => "hard",
    }
}

fn transcript_to_vtt(segments: &[TranscriptSegmentInput]) -> String {
    let mut output = String::from("WEBVTT\n\n");
    for (index, segment) in segments.iter().enumerate() {
        output.push_str(&format!(
            "{index}\n{} --> {}\n{}\n\n",
            format_timestamp(segment.start),
            format_timestamp(segment.end),
            segment.text
        ));
    }
    output
}

fn format_timestamp(seconds: f64) -> String {
    let total_millis = (seconds * 1000.0).round() as u64;
    let hours = total_millis / 3_600_000;
    let minutes = (total_millis % 3_600_000) / 60_000;
    let secs = (total_millis % 60_000) / 1000;
    let millis = total_millis % 1000;
    format!("{hours:02}:{minutes:02}:{secs:02}.{millis:03}")
}

#[cfg(test)]
mod tests {
    use super::{normalize_generated_questions, topic_transcript_excerpt};
    use crate::models::{GeneratedChoice, GeneratedQuestion, TranscriptSegmentInput};

    #[test]
    fn normalizes_generated_questions_to_objective_topic_pattern() {
        let questions = vec![
            generated_question("Explain Spark.", "essay", Some(vec![choice("A", true)])),
            generated_question("Spark is fast.", "boolean", None),
            generated_question(
                "Spark runs on the ____.",
                "short_answer",
                Some(vec![choice("A", true)]),
            ),
        ];

        let normalized = normalize_generated_questions(questions, 3, 3).unwrap();

        assert_eq!(normalized[0].question_type, "mcq");
        assert!(normalized[0].choices.is_some());
        assert_eq!(normalized[1].question_type, "true_false");
        assert!(normalized[1]
            .choices
            .as_ref()
            .is_some_and(|choices| choices.len() == 2));
        assert_eq!(
            normalized[1]
                .choices
                .as_ref()
                .map(|choices| choices[0].label.as_str()),
            Some("T")
        );
        assert_eq!(normalized[2].question_type, "completion");
        assert!(normalized[2].choices.is_none());
    }

    #[test]
    fn normalizes_missing_and_long_choice_labels_before_database_insert() {
        let questions = vec![
            generated_question(
                "Which engine is fast?",
                "mcq",
                Some(vec![
                    choice("", true),
                    choice("Option B", false),
                    choice("Long label", false),
                    choice("D", false),
                    choice("E", false),
                ]),
            ),
            generated_question(
                "Spark is fast.",
                "true_false",
                Some(vec![choice("", true), choice("False option", false)]),
            ),
            generated_question("Spark runs on the ____.", "completion", None),
        ];

        let normalized = normalize_generated_questions(questions, 3, 3).unwrap();
        let mcq_labels = normalized[0]
            .choices
            .as_ref()
            .unwrap()
            .iter()
            .map(|choice| choice.label.as_str())
            .collect::<Vec<_>>();
        let true_false_labels = normalized[1]
            .choices
            .as_ref()
            .unwrap()
            .iter()
            .map(|choice| choice.label.as_str())
            .collect::<Vec<_>>();

        assert_eq!(mcq_labels, vec!["A", "B", "C", "D"]);
        assert_eq!(true_false_labels, vec!["T", "F"]);
    }

    #[test]
    fn generated_choice_allows_missing_label_from_llm_json() {
        let question: GeneratedQuestion = serde_json::from_str(
            r#"{"stem":"Pick one","question_type":"mcq","difficulty":"easy","choices":[{"text":"Spark","is_correct":true}],"rubric":null}"#,
        )
        .unwrap();

        assert_eq!(question.choices.unwrap()[0].label, "");
    }

    #[test]
    fn rejects_wrong_generated_question_count() {
        let error =
            normalize_generated_questions(vec![generated_question("Only one.", "mcq", None)], 3, 3)
                .unwrap_err();

        assert!(error.to_string().contains("assessment questions"));
    }

    #[test]
    fn topic_transcript_excerpt_limits_context_to_topic_range() {
        let segments = vec![
            segment(0.0, 4.0, "intro"),
            segment(5.0, 9.0, "postgres indexes"),
            segment(10.0, 14.0, "query planning"),
            segment(20.0, 24.0, "unrelated outro"),
        ];

        let (text, selected) = topic_transcript_excerpt(&segments, 5.0, 14.0).unwrap();

        assert_eq!(text, "postgres indexes query planning");
        assert_eq!(selected.len(), 2);
    }

    fn generated_question(
        stem: &str,
        question_type: &str,
        choices: Option<Vec<GeneratedChoice>>,
    ) -> GeneratedQuestion {
        GeneratedQuestion {
            stem: stem.to_string(),
            question_type: question_type.to_string(),
            difficulty: String::new(),
            rubric: None,
            choices,
        }
    }

    fn choice(label: &str, is_correct: bool) -> GeneratedChoice {
        GeneratedChoice {
            label: label.to_string(),
            text: label.to_string(),
            is_correct,
        }
    }

    fn segment(start: f64, end: f64, text: &str) -> TranscriptSegmentInput {
        TranscriptSegmentInput {
            start,
            end,
            text: text.to_string(),
        }
    }
}
