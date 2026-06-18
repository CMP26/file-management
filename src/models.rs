use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VideoRecord {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub rustfs_key: String,
    pub duration_s: Option<i32>,
    pub status: String,
    pub error_msg: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CourseRecord {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
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
    pub course_id: Uuid,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MuxImportDownloadUrlRequest {
    pub title: String,
    pub course_id: Uuid,
    pub download_url: Option<String>,
    pub upload_url: Option<String>,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MuxImportDownloadUrlResponse {
    pub video_id: Uuid,
    pub course_id: Uuid,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CourseResponse {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub video_count: i64,
    pub question_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CourseListResponse {
    pub courses: Vec<CourseResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateCourseRequest {
    pub title: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SourceVideoResponse {
    pub id: Uuid,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VideoOverview {
    pub id: Uuid,
    pub course_id: Uuid,
    pub course_title: String,
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
pub struct VideoTopicResponse {
    pub id: Uuid,
    pub label: String,
    pub start_s: f64,
    pub end_s: f64,
    pub seq_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VideoDetailResponse {
    pub video: VideoOverview,
    pub topics: Vec<VideoTopicResponse>,
    pub summary: Option<String>,
    pub transcript_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeleteVideoResponse {
    pub video_id: Uuid,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TranscriptSegmentResponse {
    pub seq_index: i32,
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VideoTranscriptResponse {
    pub video_id: Uuid,
    pub full_text: Option<String>,
    pub segments: Vec<TranscriptSegmentResponse>,
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
    pub video_id: Uuid,
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
pub struct CourseRandomQuestionResponse {
    pub id: Uuid,
    pub source_video: SourceVideoResponse,
    pub topic_id: Option<Uuid>,
    pub topic_label: Option<String>,
    pub stem: String,
    pub question_type: String,
    pub difficulty: Option<String>,
    pub choices: Vec<QuestionChoiceResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CourseRandomQuestionsResponse {
    pub course_id: Uuid,
    pub requested_count: i64,
    pub questions: Vec<CourseRandomQuestionResponse>,
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
    pub status: String,
    pub is_waiting: bool,
    pub pending_count: i64,
    pub total_score: i32,
    pub breakdown: Vec<AttemptBreakdownItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JustificationResponse {
    pub answer_id: Uuid,
    pub justification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AttemptAnswerStatusItem {
    pub answer_id: Uuid,
    pub question_id: Uuid,
    pub user_answer: String,
    pub is_correct: Option<bool>,
    pub score: Option<i16>,
    pub graded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AttemptStatusResponse {
    pub attempt_id: Uuid,
    pub user_id: Uuid,
    pub video_id: Uuid,
    pub submitted_at: Option<DateTime<Utc>>,
    pub status: String,
    pub is_waiting: bool,
    pub total_score: i32,
    pub pending_count: i64,
    pub answers: Vec<AttemptAnswerStatusItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JustificationStatusResponse {
    pub answer_id: Uuid,
    pub status: String,
    pub is_waiting: bool,
    pub justification: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TranscriptChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StartTranscriptChatRequest {
    pub user_id: Uuid,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TranscriptChatRequest {
    pub user_id: Uuid,
    pub message: String,
    #[serde(default)]
    pub history: Vec<TranscriptChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TranscriptChatSource {
    pub seq_index: i32,
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TranscriptChatResponse {
    pub conversation_id: Uuid,
    pub video_id: Uuid,
    pub name: String,
    pub is_waiting: bool,
    pub user_message_id: Uuid,
    pub assistant_message_id: Option<Uuid>,
    pub answer: Option<String>,
    pub sources: Vec<TranscriptChatSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TranscriptChatMessageResponse {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub sources: Vec<TranscriptChatSource>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TranscriptChatHistoryResponse {
    pub user_id: Uuid,
    pub video_id: Uuid,
    pub video_title: String,
    pub conversation_id: Uuid,
    pub name: String,
    pub is_waiting: bool,
    pub messages: Vec<TranscriptChatMessageResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserChatConversationResponse {
    pub conversation_id: Uuid,
    pub user_id: Uuid,
    pub video_id: Uuid,
    pub video_title: String,
    pub name: String,
    pub is_waiting: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserChatListResponse {
    pub user_id: Uuid,
    pub chats: Vec<UserChatConversationResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeleteChatResponse {
    pub conversation_id: Uuid,
    pub deleted: bool,
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
    #[serde(alias = "type")]
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
