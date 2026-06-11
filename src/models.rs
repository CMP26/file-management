use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VideoRecord {
    pub id: Uuid,
    pub title: String,
    pub rustfs_key: String,
    pub duration_s: Option<i32>,
    pub status: String,
    pub error_msg: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TranscriptRecord {
    pub id: Uuid,
    pub video_id: Uuid,
    pub full_text: String,
    pub language: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TranscriptSegmentRecord {
    pub id: Uuid,
    pub transcript_id: Uuid,
    pub seq_index: i32,
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TopicRecord {
    pub id: Uuid,
    pub video_id: Uuid,
    pub label: String,
    pub start_s: f64,
    pub end_s: f64,
    pub seq_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SummaryRecord {
    pub id: Uuid,
    pub video_id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct QuestionRecord {
    pub id: Uuid,
    pub video_id: Uuid,
    pub topic_id: Option<Uuid>,
    pub stem: String,
    pub question_type: String,
    pub difficulty: Option<String>,
    pub rubric: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ChoiceRecord {
    pub id: Uuid,
    pub question_id: Uuid,
    pub label: String,
    pub text: String,
    pub is_correct: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ExamAttemptRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub video_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AttemptAnswerRecord {
    pub id: Uuid,
    pub attempt_id: Uuid,
    pub question_id: Uuid,
    pub user_answer: String,
    pub is_correct: Option<bool>,
    pub score: Option<i16>,
    pub graded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AnswerJustificationRecord {
    pub id: Uuid,
    pub attempt_answer_id: Uuid,
    pub justification: String,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UploadResponse {
    pub video_id: Uuid,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VideoOverview {
    pub id: Uuid,
    pub title: String,
    pub duration_s: Option<i32>,
    pub status: String,
    pub error_msg: Option<String>,
    pub created_at: DateTime<Utc>,
    pub topic_count: i64,
    pub question_count: i64,
    pub has_summary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VideoListResponse {
    pub videos: Vec<VideoOverview>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VideoDetailResponse {
    pub video: VideoOverview,
    pub summary: Option<String>,
    pub transcript_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LlmStatusResponse {
    pub base_url: String,
    pub configured_model: String,
    pub reachable: bool,
    pub model_ids: Vec<String>,
    pub error_msg: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuestionChoiceResponse {
    pub label: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuestionResponse {
    pub id: Uuid,
    pub stem: String,
    pub question_type: String,
    pub difficulty: Option<String>,
    pub choices: Vec<QuestionChoiceResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TopicQuestionGroupResponse {
    pub topic_id: Uuid,
    pub label: String,
    pub questions: Vec<QuestionResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuestionsByVideoResponse {
    pub video_id: Uuid,
    pub topics: Vec<TopicQuestionGroupResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StartExamRequest {
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StartExamResponse {
    pub attempt_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitAnswerInput {
    pub question_id: Uuid,
    pub user_answer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitAttemptRequest {
    pub answers: Vec<SubmitAnswerInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AttemptBreakdownItem {
    pub answer_id: Uuid,
    pub question_id: Uuid,
    pub is_correct: bool,
    pub score: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitAttemptResponse {
    pub attempt_id: Uuid,
    pub total_score: i32,
    pub breakdown: Vec<AttemptBreakdownItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JustificationResponse {
    pub answer_id: Uuid,
    pub justification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscribeResponse {
    pub full_text: String,
    pub segments: Vec<TranscriptSegmentInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegmentInput {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub seq_index: i32,
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
    pub segments: Vec<TranscriptSegmentInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicLabelResponse {
    pub label: String,
    pub start_s: f64,
    pub end_s: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedChoice {
    pub label: String,
    pub text: String,
    pub is_correct: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedQuestion {
    pub stem: String,
    pub question_type: String,
    pub difficulty: String,
    pub rubric: Option<String>,
    pub choices: Option<Vec<GeneratedChoice>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradeResponse {
    pub score: i16,
    pub is_correct: bool,
}
