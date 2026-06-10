# NexaLearn – Upload & Assessment Pipeline Design Plan

> **Stack:** Rust (backend), Python microservice (Whisper), Gemma (local via Ollama or llama.cpp),
> PostgreSQL (structured data), RustFS (object storage, S3-compatible)

---

## 1. High-Level Architecture

```
                        ┌─────────────────────────────────────────────┐
                        │              Rust Backend (Axum)             │
                        │                                              │
  HTTP Client ──────────►  /api/videos/upload                         │
                        │  /api/videos/:id/questions                  │
                        │  /api/exams/:id/submit                      │
                        │  /api/exams/:id/answers/:id/justification   │
                        └──────┬──────────────┬───────────────────────┘
                               │              │
                     job enqueue           DB / RustFS
                               │              │
                        ┌──────▼──────┐  ┌────▼──────────────┐
                        │  Tokio task  │  │   PostgreSQL       │
                        │  (worker)    │  │   RustFS (S3)      │
                        └──────┬──────┘  └───────────────────┘
                               │
          ┌────────────────────┼────────────────────┐
          │                    │                    │
   ┌──────▼──────┐    ┌────────▼──────┐    ┌───────▼───────┐
   │  Python      │    │  Gemma (local) │    │  ffmpeg        │
   │  Whisper     │    │  Ollama REST   │    │  (via Command) │
   │  microservice│    │  :11434        │    │                │
   └─────────────┘    └───────────────┘    └───────────────┘
```

---

## 2. Storage Schema

### RustFS (Object Store) – Binary / Large Files

| Key Pattern                          | Content                    |
|--------------------------------------|----------------------------|
| `videos/{video_id}/original.mp4`     | Raw uploaded video         |
| `videos/{video_id}/transcript.txt`   | Full plain-text transcript |
| `videos/{video_id}/transcript.vtt`   | Timestamped VTT transcript |

### PostgreSQL – Structured Data

```sql
-- Videos
CREATE TABLE videos (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title       TEXT NOT NULL,
    rustfs_key  TEXT NOT NULL,                 -- e.g. videos/{id}/original.mp4
    duration_s  INTEGER,
    status      TEXT NOT NULL DEFAULT 'pending',
                -- pending | transcribing | processing | ready | failed
    error_msg   TEXT,
    created_at  TIMESTAMPTZ DEFAULT now()
);

-- Transcripts
CREATE TABLE transcripts (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    video_id   UUID NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
    full_text  TEXT NOT NULL,
    language   TEXT DEFAULT 'en',
    created_at TIMESTAMPTZ DEFAULT now()
);

-- Transcript segments (timestamped chunks)
CREATE TABLE transcript_segments (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    transcript_id UUID NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
    seq_index    INTEGER NOT NULL,
    start_s      FLOAT NOT NULL,
    end_s        FLOAT NOT NULL,
    text         TEXT NOT NULL
);

-- Topics (each mapped to a transcript segment range)
CREATE TABLE topics (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    video_id    UUID NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
    label       TEXT NOT NULL,
    start_s     FLOAT NOT NULL,
    end_s       FLOAT NOT NULL,
    seq_order   INTEGER NOT NULL
);

-- Summaries
CREATE TABLE summaries (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    video_id   UUID NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
    content    TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now()
);

-- Questions
CREATE TABLE questions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    video_id    UUID NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
    topic_id    UUID REFERENCES topics(id),
    stem        TEXT NOT NULL,
    type        TEXT NOT NULL,   -- 'mcq' | 'true_false' | 'essay'
    difficulty  TEXT,            -- 'easy' | 'medium' | 'hard'
    rubric      TEXT,            -- for essay grading guidance
    created_at  TIMESTAMPTZ DEFAULT now()
);

-- Answer choices (for MCQ / true-false)
CREATE TABLE choices (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    question_id UUID NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
    label       CHAR(1) NOT NULL,  -- A, B, C, D
    text        TEXT NOT NULL,
    is_correct  BOOLEAN NOT NULL DEFAULT false
);

-- Exam attempts
CREATE TABLE exam_attempts (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID NOT NULL,
    video_id     UUID NOT NULL REFERENCES videos(id),
    started_at   TIMESTAMPTZ DEFAULT now(),
    submitted_at TIMESTAMPTZ
);

-- Per-question answers
CREATE TABLE attempt_answers (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    attempt_id   UUID NOT NULL REFERENCES exam_attempts(id) ON DELETE CASCADE,
    question_id  UUID NOT NULL REFERENCES questions(id),
    user_answer  TEXT NOT NULL,
    is_correct   BOOLEAN,
    score        SMALLINT,        -- 0–100
    graded_at    TIMESTAMPTZ
);

-- Justifications (lazy-generated, cached)
CREATE TABLE answer_justifications (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    attempt_answer_id UUID NOT NULL UNIQUE REFERENCES attempt_answers(id),
    justification     TEXT NOT NULL,
    generated_at      TIMESTAMPTZ DEFAULT now()
);
```

---

## 3. Rust Crate & Module Layout

```
nexalearn-backend/
├── Cargo.toml
└── src/
    ├── main.rs                    ← Axum router, startup
    ├── config.rs                  ← env config (DB url, RustFS url, Gemma url, Whisper url)
    │
    ├── db/
    │   ├── mod.rs
    │   └── pool.rs                ← sqlx PgPool setup
    │
    ├── storage/
    │   ├── mod.rs
    │   └── rustfs.rs              ← aws_sdk_s3 client pointed at RustFS endpoint
    │
    ├── llm/
    │   ├── mod.rs
    │   └── gemma.rs               ← HTTP client to Ollama / llama.cpp REST
    │
    ├── whisper/
    │   ├── mod.rs
    │   └── client.rs              ← HTTP client to Python Whisper microservice
    │
    ├── ingestion/
    │   ├── mod.rs
    │   ├── handler.rs             ← POST /api/videos/upload (Axum handler)
    │   ├── worker.rs              ← async task: orchestrates all pipeline steps
    │   ├── audio.rs               ← ffmpeg child process: mp4 → wav
    │   ├── segmenter.rs           ← chunk transcript into ~300-token windows
    │   ├── topic_labeler.rs       ← Gemma: label each chunk
    │   ├── question_gen.rs        ← Gemma: generate questions per topic
    │   └── summarizer.rs          ← Gemma: full-video summary
    │
    └── assessment/
        ├── mod.rs
        ├── handler.rs             ← GET questions, POST submit, GET justification
        ├── grader.rs              ← deterministic MCQ + LLM essay grading
        └── justifier.rs           ← lazy justification generation + cache lookup
```

### Key Rust Dependencies (Cargo.toml)

```toml
[dependencies]
axum            = "0.7"
tokio           = { version = "1", features = ["full"] }
sqlx             = { version = "0.7", features = ["postgres", "uuid", "chrono", "runtime-tokio-rustls"] }
aws-sdk-s3      = "1"                   # RustFS is S3-compatible
aws-config      = "1"
reqwest         = { version = "0.11", features = ["json"] }
serde            = { version = "1", features = ["derive"] }
serde_json       = "1"
uuid             = { version = "1", features = ["v4", "serde"] }
tokio-util       = "0.7"
multipart        = "0.18"               # or axum-multipart
tracing          = "0.1"
tracing-subscriber = "0.3"
thiserror        = "1"
anyhow           = "1"
```

---

## 4. Pipeline 1 — Ingestion

### 4.1 HTTP Upload Handler (`ingestion/handler.rs`)

```
POST /api/videos/upload
Content-Type: multipart/form-data

Fields:
  - file: <video binary>
  - title: string

Response 202:
  { "video_id": "<uuid>", "status": "pending" }
```

**Handler logic (sync part):**
1. Parse multipart body, extract file bytes + title
2. Generate `video_id` (UUID v4)
3. Upload raw bytes → RustFS at `videos/{video_id}/original.mp4`
4. Insert row into `videos` table (`status = 'pending'`)
5. Spawn Tokio background task (the worker) with `video_id`
6. Return `202 Accepted` immediately

### 4.2 Worker (`ingestion/worker.rs`)

The worker runs as a `tokio::spawn` task. Each step updates `videos.status` so progress can be polled.

```
Step 1 │ status = 'extracting_audio'
       │ ffmpeg: download video from RustFS to /tmp/{video_id}.mp4
       │       → extract audio → /tmp/{video_id}.wav
       │
Step 2 │ status = 'transcribing'
       │ POST to Python Whisper service with wav file
       │ receive: { full_text, segments: [{start, end, text}] }
       │ store:
       │   → transcripts table (full_text)
       │   → transcript_segments table (all segments)
       │   → upload transcript.txt + transcript.vtt to RustFS
       │
Step 3 │ status = 'labeling_topics'
       │ call segmenter.rs: group transcript_segments into ~300-token chunks
       │ for each chunk → Gemma: produce topic label + timestamps
       │ insert into topics table
       │
Step 4 │ status = 'generating_questions'
       │ for each topic → Gemma: generate N questions as JSON
       │ parse JSON → insert questions + choices into DB
       │
Step 5 │ status = 'summarizing'
       │ Gemma: summarize full transcript
       │ insert into summaries table
       │
Step 6 │ status = 'ready'
       │ clean up /tmp files
```

**Error handling:** any step failure sets `status = 'failed'` + writes `error_msg`. The worker can be re-triggered via a retry endpoint.

### 4.3 Audio Extraction (`ingestion/audio.rs`)

```rust
// Uses tokio::process::Command
pub async fn extract_audio(video_path: &Path, out_path: &Path) -> Result<()> {
    Command::new("ffmpeg")
        .args(["-i", video_path.to_str().unwrap(),
               "-ar", "16000",       // 16kHz — Whisper prefers this
               "-ac", "1",           // mono
               "-f", "wav",
               out_path.to_str().unwrap()])
        .output()
        .await?;
    Ok(())
}
```

### 4.4 Transcript Segmenter (`ingestion/segmenter.rs`)

Groups `transcript_segments` rows into topic-sized chunks:
- Target: ~300 tokens per chunk (approximate by word count × 1.3)
- Never split mid-sentence — respect segment boundaries
- Output: `Vec<Chunk>` where each chunk holds its segments + merged text

### 4.5 Gemma Client (`llm/gemma.rs`)

```rust
pub struct GemmaClient {
    base_url: String,   // e.g. http://localhost:11434
    model: String,      // e.g. "gemma3"
    client: reqwest::Client,
}

impl GemmaClient {
    pub async fn generate(&self, prompt: &str) -> Result<String> {
        // POST /api/generate  (Ollama)
        // or POST /v1/chat/completions  (llama.cpp)
    }

    // Convenience: call generate(), attempt JSON parse, retry up to 3× on parse fail
    pub async fn generate_json<T: DeserializeOwned>(&self, prompt: &str) -> Result<T> { ... }
}
```

### 4.6 Topic Labeling Prompt

```
Given this video transcript segment, return ONLY valid JSON (no markdown).

Transcript:
{chunk_text}

JSON schema:
{
  "label": "short topic name (3-6 words)",
  "start_s": <float>,
  "end_s": <float>
}
```

### 4.7 Question Generation Prompt

```
You are creating educational questions for a learning platform.

Topic: {topic_label}
Transcript segment:
{chunk_text}

Generate {n} questions. Return ONLY valid JSON array (no markdown):
[
  {
    "stem": "question text",
    "type": "mcq" | "true_false" | "essay",
    "difficulty": "easy" | "medium" | "hard",
    "rubric": "grading guidance (for essay only, else null)",
    "choices": [                         // null for essay
      { "label": "A", "text": "...", "is_correct": false },
      { "label": "B", "text": "...", "is_correct": true },
      ...
    ]
  }
]
```

### 4.8 Summary Prompt

```
Summarize the following video transcript for a student in 3-5 clear paragraphs.
Focus on key concepts, not timestamps.

Transcript:
{full_text}
```

---

## 5. Python Whisper Microservice

A minimal FastAPI service — kept entirely separate from the Rust backend.

### Interface

```
POST /transcribe
Content-Type: multipart/form-data
  file: <wav binary>
  language: "en"   (optional)

Response 200:
{
  "full_text": "...",
  "segments": [
    { "start": 0.0, "end": 3.4, "text": "Hello and welcome..." },
    ...
  ]
}
```

### Implementation sketch

```python
# main.py
from fastapi import FastAPI, UploadFile
import whisper, tempfile, os

app = FastAPI()
model = whisper.load_model("base")   # or "small", "medium"

@app.post("/transcribe")
async def transcribe(file: UploadFile, language: str = "en"):
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp:
        tmp.write(await file.read())
        tmp_path = tmp.name
    result = model.transcribe(tmp_path, language=language)
    os.unlink(tmp_path)
    return {
        "full_text": result["text"],
        "segments": [
            {"start": s["start"], "end": s["end"], "text": s["text"]}
            for s in result["segments"]
        ]
    }
```

### Whisper Client in Rust (`whisper/client.rs`)

```rust
pub struct WhisperClient {
    base_url: String,
    client: reqwest::Client,
}

pub struct TranscriptResponse {
    pub full_text: String,
    pub segments: Vec<Segment>,
}

impl WhisperClient {
    // Reads wav bytes from disk, sends multipart POST, returns TranscriptResponse
    pub async fn transcribe(&self, wav_path: &Path, language: &str) -> Result<TranscriptResponse>
}
```

---

## 6. Pipeline 2 — Assessment API

### 6.1 Endpoints

#### Get questions for a video

```
GET /api/videos/:video_id/questions
  ?topic_id=<uuid>        (optional filter)
  ?type=mcq|essay|...     (optional filter)

Response 200:
{
  "video_id": "...",
  "topics": [
    {
      "topic_id": "...",
      "label": "...",
      "questions": [
        {
          "id": "...",
          "stem": "...",
          "type": "mcq",
          "difficulty": "medium",
          "choices": [{ "label": "A", "text": "...", "is_correct": false }, ...]
        }
      ]
    }
  ]
}
```

> Note: `is_correct` is stripped from the response when serving to students.
> Only returned to teachers or after attempt submission.

#### Start an exam attempt

```
POST /api/videos/:video_id/exams/start
Body: { "user_id": "..." }

Response 201:
{ "attempt_id": "..." }
```

#### Submit answers

```
POST /api/exams/:attempt_id/submit
Body:
{
  "answers": [
    { "question_id": "...", "user_answer": "B" },
    { "question_id": "...", "user_answer": "essay text here..." }
  ]
}

Response 200:
{
  "attempt_id": "...",
  "total_score": 74,
  "breakdown": [
    {
      "answer_id": "...",
      "question_id": "...",
      "is_correct": true,
      "score": 100
    },
    {
      "answer_id": "...",
      "question_id": "...",
      "is_correct": false,
      "score": 40
    }
  ]
}
```

#### Get justification (on-demand, cached)

```
GET /api/exams/:attempt_id/answers/:answer_id/justification

Response 200 (cached or freshly generated):
{
  "answer_id": "...",
  "justification": "Your answer was partially correct. You correctly identified X,
                    but missed Y because Z. The key concept here is..."
}
```

### 6.2 Grader (`assessment/grader.rs`)

```
For MCQ / True-False:
  → Look up correct choice in DB
  → Compare to user_answer string (case-insensitive label match)
  → score = 100 if correct, 0 if not
  → No LLM call

For Essay:
  → Build Gemma grading prompt (see below)
  → Parse JSON response: { score: 0-100, is_correct: bool }
  → Store result
```

**Essay grading prompt:**

```
You are a strict academic grader. Return ONLY valid JSON (no markdown).

Question: {stem}
Grading rubric: {rubric}
Student answer: {user_answer}

JSON schema:
{
  "score": <integer 0-100>,
  "is_correct": <boolean, true if score >= 60>
}
```

### 6.3 Justifier (`assessment/justifier.rs`)

```
1. SELECT from answer_justifications WHERE attempt_answer_id = $1
   → if found: return cached justification

2. Fetch: question stem, rubric, correct answer text, user_answer, score

3. Build prompt (see below) → call Gemma → get plain text response

4. INSERT into answer_justifications (attempt_answer_id, justification)

5. Return justification text
```

**Justification prompt:**

```
You are a helpful tutor explaining exam feedback to a student.

Question: {stem}
Correct answer / rubric: {correct_answer_or_rubric}
Student answered: {user_answer}
Score given: {score}/100

In 2-4 sentences:
- Tell the student what they got right
- Tell the student what they missed or got wrong
- Give one concrete tip for improvement
Do not repeat the question. Write directly to the student.
```

---

## 7. RustFS Client (`storage/rustfs.rs`)

RustFS is S3-compatible, so use `aws-sdk-s3` pointed at the RustFS endpoint.

```rust
pub struct RustFsClient {
    inner: aws_sdk_s3::Client,
    bucket: String,
}

impl RustFsClient {
    pub async fn upload(&self, key: &str, data: Vec<u8>, content_type: &str) -> Result<()>
    pub async fn download(&self, key: &str) -> Result<Vec<u8>>
    pub async fn presigned_url(&self, key: &str, expires_in: Duration) -> Result<String>
}
```

Config (via env):
```
RUSTFS_ENDPOINT=http://localhost:9000
RUSTFS_ACCESS_KEY=...
RUSTFS_SECRET_KEY=...
RUSTFS_BUCKET=nexalearn
```

---

## 8. Configuration (`config.rs`)

```rust
pub struct Config {
    pub database_url: String,         // postgres://...
    pub rustfs_endpoint: String,      // http://localhost:9000
    pub rustfs_bucket: String,
    pub rustfs_access_key: String,
    pub rustfs_secret_key: String,
    pub gemma_base_url: String,       // http://localhost:11434
    pub gemma_model: String,          // gemma3
    pub whisper_url: String,          // http://localhost:8000
    pub tmp_dir: String,              // /tmp/nexalearn
}
```

Load via `std::env` or a `.env` file with the `dotenvy` crate.

---

## 9. Error Handling Strategy

- Use `thiserror` for typed domain errors per module
- All worker steps are wrapped in `match` — any `Err` updates `videos.status = 'failed'`
- LLM JSON parse failures: retry up to 3× with the same prompt before failing
- Whisper service unavailable: fail fast, set video status = 'failed', return descriptive error_msg
- All HTTP handlers return structured JSON errors:

```json
{ "error": "video_not_found", "message": "No video with id ..." }
```

---

## 10. Development Startup Checklist

```
1. Start PostgreSQL               → run migrations (sqlx migrate run)
2. Start RustFS                   → docker run rustfs/rustfs ...
3. Start Ollama + pull Gemma      → ollama serve && ollama pull gemma3
4. Start Python Whisper service   → uvicorn main:app --port 8000
5. Start Rust backend             → cargo run
```

---

## 11. Sequence Diagrams

### Upload Flow

```
Client          Rust Backend       RustFS       Whisper Svc     Gemma (Ollama)    PostgreSQL
  │                   │               │               │                │               │
  │ POST /upload      │               │               │                │               │
  ├──────────────────►│               │               │                │               │
  │                   │ upload mp4    │               │                │               │
  │                   ├──────────────►│               │                │               │
  │                   │ INSERT video  │               │                │               │
  │                   ├───────────────┼───────────────┼────────────────┼──────────────►│
  │ 202 { video_id }  │               │               │                │               │
  ◄───────────────────┤               │               │                │               │
  │                   │ [background worker starts]    │                │               │
  │                   │ ffmpeg wav    │               │                │               │
  │                   │──────(local)──┤               │                │               │
  │                   │ POST /transcribe              │                │               │
  │                   ├───────────────┼──────────────►│                │               │
  │                   │ segments[]    │               │                │               │
  │                   ◄───────────────┼───────────────┤                │               │
  │                   │ store transcript              │                │               │
  │                   ├───────────────┼───────────────┼────────────────┼──────────────►│
  │                   │ topic labels (per chunk)      │                │               │
  │                   ├───────────────┼───────────────┼───────────────►│               │
  │                   │ questions JSON (per topic)    │                │               │
  │                   ├───────────────┼───────────────┼───────────────►│               │
  │                   │ summary text  │               │                │               │
  │                   ├───────────────┼───────────────┼───────────────►│               │
  │                   │ store all     │               │                │               │
  │                   ├───────────────┼───────────────┼────────────────┼──────────────►│
  │                   │ status = 'ready'              │                │               │
```

### Submit + Justification Flow

```
Client          Rust Backend         Gemma           PostgreSQL
  │                   │                │                 │
  │ POST /submit      │                │                 │
  ├──────────────────►│                │                 │
  │                   │ fetch correct answers            │
  │                   ├─────────────────────────────────►│
  │                   │ grade MCQ (local, no LLM)        │
  │                   │ grade essay → prompt             │
  │                   ├───────────────►│                 │
  │                   │ { score, is_correct }            │
  │                   ◄───────────────┤                 │
  │                   │ store attempt_answers            │
  │                   ├─────────────────────────────────►│
  │ { breakdown[] }   │                │                 │
  ◄───────────────────┤                │                 │
  │                   │                │                 │
  │ GET /justification│                │                 │
  ├──────────────────►│                │                 │
  │                   │ check cache    │                 │
  │                   ├─────────────────────────────────►│
  │                   │ (miss) → prompt Gemma            │
  │                   ├───────────────►│                 │
  │                   │ justification text               │
  │                   ◄───────────────┤                 │
  │                   │ store in answer_justifications   │
  │                   ├─────────────────────────────────►│
  │ { justification } │                │                 │
  ◄───────────────────┤                │                 │
```
