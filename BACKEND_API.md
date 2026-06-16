# Backend API

Base URL for local development:

```text
http://localhost:8080
```

Interactive OpenAPI documentation is served at:

```text
http://localhost:8080/swagger-ui
```

Errors are returned as JSON:

```json
{
  "error": "not_found",
  "message": "not found: video <id> was not found"
}
```

Common error codes are `bad_request`, `not_found`, `conflict`, `external_service_error`, `database_error`, `io_error`, `http_error`, `json_error`, and `internal_error`.

## Health

### `GET /healthz`

Returns a plain-text health check.

```bash
curl http://localhost:8080/healthz
```

Response:

```text
ok
```

## LLM

The backend serializes Gemma generation requests by default to protect local llama.cpp/Gemma servers from dropped concurrent requests. Tune this with `GEMMA_MAX_CONCURRENT_REQUESTS` (default `1`) and `GEMMA_REQUEST_TIMEOUT_SECONDS` (default `300`).

### `GET /api/llm/status`

Checks whether the backend can reach the configured OpenAI-compatible Gemma server.

```bash
curl http://localhost:8080/api/llm/status
```

Response:

```json
{
  "base_url": "http://localhost:8100",
  "configured_model": "ggml-org/gemma-4-E4B-it-GGUF",
  "reachable": true,
  "model_ids": ["ggml-org/gemma-4-E4B-it-GGUF"],
  "error_msg": null
}
```

## Courses

### `GET /api/courses`

Lists courses with video and generated-question counts.

```bash
curl http://localhost:8080/api/courses
```

Response:

```json
{
  "courses": [
    {
      "id": "7e9ceae3-6ab9-45dc-8f3d-b64df2c103669",
      "title": "Biology 101",
      "description": "Introductory biology lectures",
      "created_at": "2026-06-13T09:30:00Z",
      "video_count": 3,
      "question_count": 18
    }
  ]
}
```

### `POST /api/courses`

Creates a course that videos can be attached to.

```bash
curl -X POST http://localhost:8080/api/courses \
  -H "content-type: application/json" \
  -d '{
    "title": "Biology 101",
    "description": "Introductory biology lectures"
  }'
```

Response:

```json
{
  "id": "7e9ceae3-6ab9-45dc-8f3d-b64df2c103669",
  "title": "Biology 101",
  "description": "Introductory biology lectures",
  "created_at": "2026-06-13T09:30:00Z",
  "video_count": 0,
  "question_count": 0
}
```

## Videos

Videos belong to a course through `videos.course_id`. Questions keep their source video through `questions.video_id`, so a course question can always be traced back to the video and transcript used for grading and justification.

### `GET /api/videos`

Lists uploaded videos and their processing status.

```bash
curl http://localhost:8080/api/videos
```

Response:

```json
{
  "videos": [
    {
      "id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
      "course_id": "7e9ceae3-6ab9-45dc-8f3d-b64df2c103669",
      "course_title": "Biology 101",
      "title": "Demo lecture",
      "duration_s": 620,
      "status": "ready",
      "error_msg": null,
      "created_at": "2026-06-13T10:00:00Z",
      "topic_count": 4,
      "question_count": 12,
      "has_summary": true
    }
  ]
}
```

Status values used by the ingestion worker include `pending`, `extracting_audio`, `transcribing`, `labeling_topics`, `generating_questions`, `summarizing`, `ready`, and `failed`.

### `POST /api/videos/upload`

Uploads a video or audio file and starts ingestion in the background.

Request body is `multipart/form-data`.

| Field | Type | Required | Notes |
|---|---|---:|---|
| `course_id` | UUID | yes | Course this source video belongs to |
| `title` | string | yes | Display title for the uploaded media |
| `file` | file | yes | Video or audio file, up to the backend body limit |

```bash
curl -F "course_id=7e9ceae3-6ab9-45dc-8f3d-b64df2c103669" \
  -F "title=Demo lecture" \
  -F "file=@sample.mp4" \
  http://localhost:8080/api/videos/upload
```

Response:

```json
{
  "video_id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
  "course_id": "7e9ceae3-6ab9-45dc-8f3d-b64df2c103669",
  "status": "pending"
}
```

## Mux Import

### `POST /api/mux/import-upload-url`

Imports a video from a URL provided by your Mux-facing backend, stores it in RustFS, creates a normal `videos` row, and starts the same transcription/question-generation pipeline used by multipart uploads.

This endpoint expects a URL that the backend can `GET` to download the media bytes. It does not create a Mux direct-upload URL for browser uploads.

Request:

```json
{
  "title": "Mux uploaded lecture",
  "course_id": "7e9ceae3-6ab9-45dc-8f3d-b64df2c103669",
  "upload_url": "https://example.com/path/to/video.mp4",
  "file_name": "lecture.mp4"
}
```

`file_name` is optional. When it is omitted, the backend infers the stored file extension from the URL path or `content-type` response header.

```bash
curl -X POST http://localhost:8080/api/mux/import-upload-url \
  -H "content-type: application/json" \
  -d '{
    "title": "Mux uploaded lecture",
    "course_id": "7e9ceae3-6ab9-45dc-8f3d-b64df2c103669",
    "upload_url": "https://example.com/path/to/video.mp4",
    "file_name": "lecture.mp4"
  }'
```

Response:

```json
{
  "video_id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
  "course_id": "7e9ceae3-6ab9-45dc-8f3d-b64df2c103669",
  "status": "pending"
}
```

The endpoint accepts `http` and `https` URLs and rejects remote media larger than 1 GiB.

### `GET /api/videos/{video_id}`

Returns one video's processing state, generated topics, latest summary, and transcript preview.

```bash
curl http://localhost:8080/api/videos/3aa9f8b2-cab5-41f6-9024-2b91533d1db0
```

Response:

```json
{
  "video": {
    "id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
    "course_id": "7e9ceae3-6ab9-45dc-8f3d-b64df2c103669",
    "course_title": "Biology 101",
    "title": "Demo lecture",
    "duration_s": 620,
    "status": "ready",
    "error_msg": null,
    "created_at": "2026-06-13T10:00:00Z",
    "topic_count": 4,
    "question_count": 12,
    "has_summary": true
  },
  "topics": [
    {
      "id": "c095fe07-f23d-4120-8af2-57f8b319a7ab",
      "label": "Introduction",
      "start_s": 0.0,
      "end_s": 95.4,
      "seq_order": 0
    }
  ],
  "summary": "The lecture introduces...",
  "transcript_preview": "Welcome to..."
}
```

### `DELETE /api/videos/{video_id}`

Deletes the video row, generated database records, and stored objects.

```bash
curl -X DELETE http://localhost:8080/api/videos/3aa9f8b2-cab5-41f6-9024-2b91533d1db0
```

Response:

```json
{
  "video_id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
  "deleted": true
}
```

### `GET /api/videos/{video_id}/media`

Streams the browser-friendly `playback.mp4` when available, otherwise falls back to the original uploaded media. Supports HTTP range requests from the browser video player.

```bash
curl -L http://localhost:8080/api/videos/3aa9f8b2-cab5-41f6-9024-2b91533d1db0/media --output media.mp4
```

### `GET /api/videos/{video_id}/transcript`

Returns the latest transcript text and timestamped segments. If the video exists but transcription has not completed, `full_text` is `null` and `segments` is empty.

```bash
curl http://localhost:8080/api/videos/3aa9f8b2-cab5-41f6-9024-2b91533d1db0/transcript
```

Response:

```json
{
  "video_id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
  "full_text": "Welcome to the lecture...",
  "segments": [
    {
      "seq_index": 0,
      "start_s": 0.0,
      "end_s": 4.2,
      "text": "Welcome to the lecture."
    }
  ]
}
```

### `GET /api/videos/{video_id}/transcript.vtt`

Returns WebVTT captions for the video player.

```bash
curl http://localhost:8080/api/videos/3aa9f8b2-cab5-41f6-9024-2b91533d1db0/transcript.vtt
```

## Transcript Chat

Transcript chat is session based. A user starts a named chat for a video, sends messages to that chat, retrieves the chat later by user and chat id, lists all chats for a user, and can delete a chat. Each chat keeps its own message history and uses that history as part of the model context.

The frontend currently uses this fixed test user id:

```text
11111111-1111-4111-8111-111111111111
```

### `POST /api/videos/{video_id}/chats`

Starts a named chat session for a video.

Request:

```json
{
  "user_id": "11111111-1111-4111-8111-111111111111",
  "name": "Exam prep questions"
}
```

```bash
curl -X POST http://localhost:8080/api/videos/3aa9f8b2-cab5-41f6-9024-2b91533d1db0/chats \
  -H "content-type: application/json" \
  -d '{
    "user_id": "11111111-1111-4111-8111-111111111111",
    "name": "Exam prep questions"
  }'
```

Response:

```json
{
  "user_id": "11111111-1111-4111-8111-111111111111",
  "video_id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
  "video_title": "Demo lecture",
  "conversation_id": "97fa16f9-d84a-47ef-a752-8a5563c144cf",
  "name": "Exam prep questions",
  "is_waiting": false,
  "messages": []
}
```

### `POST /api/chats/{conversation_id}/messages`

Saves the user's message immediately, marks the chat as waiting for the LLM response, waits for a slot in the backend's LLM queue, asks the local Gemma model, then saves the assistant response and clears the waiting state. If the same chat is already waiting, the endpoint returns `409 Conflict` instead of starting a competing LLM request.

When transcript segments are available, the backend selects relevant transcript context and returns those source segments. If the transcript is missing or the question is outside the video, the model can still answer using broader knowledge and should label that as outside-video context. The chat's stored messages are used as that chat's history/context.

While the model is still generating, `GET /api/users/{user_id}/chats` and `GET /api/users/{user_id}/chats/{conversation_id}` can show `is_waiting: true`; the submitted user message is already saved at that point. Stale waiting states older than the configured LLM timeout window are cleared when chats are read or a new message is submitted.

Request:

```json
{
  "user_id": "11111111-1111-4111-8111-111111111111",
  "message": "What is the main idea of the introduction?",
  "history": []
}
```

`history` is optional and only used as a fallback before the chat has stored messages. Supported roles are `user` and `assistant`; unknown roles are treated as `user` when building the prompt.

```bash
curl -X POST http://localhost:8080/api/chats/97fa16f9-d84a-47ef-a752-8a5563c144cf/messages \
  -H "content-type: application/json" \
  -d '{
    "user_id": "11111111-1111-4111-8111-111111111111",
    "message": "What should I remember from the introduction?",
    "history": []
  }'
```

Response:

```json
{
  "conversation_id": "97fa16f9-d84a-47ef-a752-8a5563c144cf",
  "video_id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
  "name": "Exam prep questions",
  "is_waiting": false,
  "user_message_id": "35e192cf-b493-4b82-b3a2-b5a8dd5060dd",
  "assistant_message_id": "c460d657-3ac1-4ba1-a384-b3d818afac59",
  "answer": "The introduction frames the lecture around...",
  "sources": [
    {
      "seq_index": 0,
      "start_s": 0.0,
      "end_s": 4.2,
      "text": "Welcome to the lecture."
    }
  ]
}
```

### `GET /api/users/{user_id}/chats`

Lists saved chats for a user. Add `?video_id=<video-id>` to filter to chats for one video.

```bash
curl "http://localhost:8080/api/users/11111111-1111-4111-8111-111111111111/chats?video_id=3aa9f8b2-cab5-41f6-9024-2b91533d1db0"
```

Response:

```json
{
  "user_id": "11111111-1111-4111-8111-111111111111",
  "chats": [
    {
      "conversation_id": "97fa16f9-d84a-47ef-a752-8a5563c144cf",
      "user_id": "11111111-1111-4111-8111-111111111111",
      "video_id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
      "video_title": "Demo lecture",
      "name": "Exam prep questions",
      "is_waiting": false,
      "created_at": "2026-06-13T10:15:00Z",
      "updated_at": "2026-06-13T10:18:00Z",
      "message_count": 2
    }
  ]
}
```

### `GET /api/users/{user_id}/chats/{conversation_id}`

Retrieves one chat, including its name, video context, and preserved message history.

```bash
curl http://localhost:8080/api/users/11111111-1111-4111-8111-111111111111/chats/97fa16f9-d84a-47ef-a752-8a5563c144cf
```

Response:

```json
{
  "user_id": "11111111-1111-4111-8111-111111111111",
  "video_id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
  "video_title": "Demo lecture",
  "conversation_id": "97fa16f9-d84a-47ef-a752-8a5563c144cf",
  "name": "Exam prep questions",
  "is_waiting": false,
  "messages": [
    {
      "id": "35e192cf-b493-4b82-b3a2-b5a8dd5060dd",
      "role": "user",
      "content": "What should I remember from the introduction?",
      "sources": [],
      "created_at": "2026-06-13T10:17:00Z"
    },
    {
      "id": "c460d657-3ac1-4ba1-a384-b3d818afac59",
      "role": "assistant",
      "content": "The introduction frames the lecture around...",
      "sources": [
        {
          "seq_index": 0,
          "start_s": 0.0,
          "end_s": 4.2,
          "text": "Welcome to the lecture."
        }
      ],
      "created_at": "2026-06-13T10:17:03Z"
    }
  ]
}
```

### `DELETE /api/users/{user_id}/chats/{conversation_id}`

Deletes one chat and its messages.

```bash
curl -X DELETE http://localhost:8080/api/users/11111111-1111-4111-8111-111111111111/chats/97fa16f9-d84a-47ef-a752-8a5563c144cf
```

Response:

```json
{
  "conversation_id": "97fa16f9-d84a-47ef-a752-8a5563c144cf",
  "deleted": true
}
```

## Assessment

### `GET /api/courses/{course_id}/questions/random`

Returns a requested number of random questions generated from videos in one course. Each returned question includes `source_video`, which is the video whose transcript/rubric should be used for grading and justification.

Optional query parameters:

| Name | Type | Notes |
|---|---|---|
| `count` | integer | Number of random questions to return. Defaults to `10`; maximum is `100` |
| `type` | string | Filter by question type, such as `mcq`, `true_false`, or `essay` |

```bash
curl "http://localhost:8080/api/courses/7e9ceae3-6ab9-45dc-8f3d-b64df2c103669/questions/random?count=5&type=essay"
```

Response:

```json
{
  "course_id": "7e9ceae3-6ab9-45dc-8f3d-b64df2c103669",
  "requested_count": 5,
  "questions": [
    {
      "id": "c84ec1f7-a9e8-4018-97d7-a9fef79040b9",
      "source_video": {
        "id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
        "title": "Demo lecture"
      },
      "topic_id": "c095fe07-f23d-4120-8af2-57f8b319a7ab",
      "topic_label": "Introduction",
      "stem": "What is the lecture about?",
      "question_type": "essay",
      "difficulty": "medium",
      "choices": []
    }
  ]
}
```

### `GET /api/videos/{video_id}/questions`

Returns generated questions grouped by topic.

Optional query parameters:

| Name | Type | Notes |
|---|---|---|
| `topic_id` | UUID | Return questions for one topic |
| `type` | string | Filter by question type, such as `mcq`, `true_false`, or `essay` |

```bash
curl "http://localhost:8080/api/videos/3aa9f8b2-cab5-41f6-9024-2b91533d1db0/questions?type=mcq"
```

Response:

```json
{
  "video_id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
  "topics": [
    {
      "topic_id": "c095fe07-f23d-4120-8af2-57f8b319a7ab",
      "label": "Introduction",
      "questions": [
        {
          "id": "c84ec1f7-a9e8-4018-97d7-a9fef79040b9",
          "video_id": "3aa9f8b2-cab5-41f6-9024-2b91533d1db0",
          "stem": "What is the lecture about?",
          "question_type": "mcq",
          "difficulty": "easy",
          "choices": [
            { "label": "A", "text": "Topic modeling" },
            { "label": "B", "text": "Storage engines" }
          ]
        }
      ]
    }
  ]
}
```

### `POST /api/videos/{video_id}/exams/start`

Starts an exam attempt for a user.

```bash
curl -X POST http://localhost:8080/api/videos/3aa9f8b2-cab5-41f6-9024-2b91533d1db0/exams/start \
  -H "content-type: application/json" \
  -d '{ "user_id": "2a58cc88-5cb6-432d-bcaf-4ff12c010e3b" }'
```

Response:

```json
{
  "attempt_id": "11da48d4-f5da-4702-9e94-4faed5dbe2f2"
}
```

### `POST /api/exams/{attempt_id}/submit`

Submits answers and grades the attempt. MCQ and true/false answers are graded from stored choices; free-form answers are graded through the local LLM. Every submitted question must belong to the attempt video.

```bash
curl -X POST http://localhost:8080/api/exams/11da48d4-f5da-4702-9e94-4faed5dbe2f2/submit \
  -H "content-type: application/json" \
  -d '{
    "answers": [
      {
        "question_id": "c84ec1f7-a9e8-4018-97d7-a9fef79040b9",
        "user_answer": "A"
      }
    ]
  }'
```

Response:

```json
{
  "attempt_id": "11da48d4-f5da-4702-9e94-4faed5dbe2f2",
  "total_score": 1,
  "breakdown": [
    {
      "answer_id": "25cb9158-f0d7-47e8-93bb-6817540a16dc",
      "question_id": "c84ec1f7-a9e8-4018-97d7-a9fef79040b9",
      "is_correct": true,
      "score": 1
    }
  ]
}
```

### `GET /api/exams/{attempt_id}/answers/{answer_id}/justification`

Returns or creates an LLM-generated justification for a graded answer.

```bash
curl http://localhost:8080/api/exams/11da48d4-f5da-4702-9e94-4faed5dbe2f2/answers/25cb9158-f0d7-47e8-93bb-6817540a16dc/justification
```

Response:

```json
{
  "answer_id": "25cb9158-f0d7-47e8-93bb-6817540a16dc",
  "justification": "The selected answer matches the transcript because..."
}
