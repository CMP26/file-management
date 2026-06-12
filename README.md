# NexaLearn local pipeline

This repository contains a local-first implementation of the upload, transcription, question generation, grading, and justification pipeline described in `nexalearn_pipeline_design.md`.

The default development stack is Docker Compose so the full system can be started with one command. The compose file wires together:

- PostgreSQL for structured data
- An S3-compatible object store exposed as `rustfs`
- A Whisper-compatible Python service for transcription
- The Rust Axum backend

The backend is configured to call your host-running llama.cpp server with Gemma on port `8100` using the OpenAI-compatible `/v1/chat/completions` API. Compose adds the Docker host gateway so the backend container can reach that endpoint directly.

The Whisper service is a real local transcription container using `faster-whisper`. It now accepts uploaded media, normalizes it through `ffmpeg`, and returns transcript text plus segments, which makes it usable with real video or audio files instead of only prebuilt WAV files.

## Local setup

The recommended local workflow is hybrid:

- Run `llama-server` on the host.
- Run the Rust backend on the host.
- Run PostgreSQL, MinIO/RustFS, and Whisper in Docker Compose.

This avoids the fragile part of Docker networking: a container trying to call an LLM server bound to host `localhost`. The backend and llama both run on the host, so `GEMMA_BASE_URL=http://localhost:8100` works directly.

### Prerequisites

Install these on the host machine:

- Docker with Docker Compose
- Rust/Cargo
- `ffmpeg`
- `llama-server`
- Your Gemma GGUF model

Docker still runs PostgreSQL, MinIO/RustFS, and Whisper, so you do not need to install those locally.

### 1. Start llama on the host

Use your existing command:

```bash
llama-server \
  -m ~/.cache/huggingface/hub/models--ggml-org--gemma-4-E4B-it-GGUF/snapshots/*/gemma-4-E4B-it-Q4_K_M.gguf \
  --alias "ggml-org/gemma-4-E4B-it-GGUF" \
  --port 8100 \
  --ctx-size 65536 \
  --reasoning-budget 512 \
  -t 8
```

For the hybrid workflow, `--host 0.0.0.0` is not required because the backend also runs on the host. Check llama before continuing:

```bash
curl http://localhost:8100/v1/models
```

### 2. Start Docker infrastructure

In a second terminal:

```bash
./scripts/start-infra.sh
```

This starts only:

- PostgreSQL on `localhost:5432`
- MinIO/RustFS on `localhost:9000` and console on `localhost:9001`
- Whisper on `localhost:8000`

It intentionally does not start the Dockerized backend, because the backend will run on the host.

### 3. Start the backend on the host

In a third terminal:

```bash
./scripts/run-host-backend.sh
```

The script sets the local development environment:

```text
DATABASE_URL=postgres://nexalearn:nexalearn@localhost:5432/nexalearn
RUSTFS_ENDPOINT=http://localhost:9000
WHISPER_URL=http://localhost:8000
GEMMA_BASE_URL=http://localhost:8100
GEMMA_MODEL=ggml-org/gemma-4-E4B-it-GGUF
BIND_ADDR=127.0.0.1:8080
```

The backend runs migrations automatically on startup.

### 4. Open the frontend

Open:

```text
http://localhost:8080/
```

Use the console to upload media, refresh videos, inspect processing status, load questions, start an attempt, submit answers, and request justifications.

The selected-video panel also includes:

- A video player for the uploaded media
- Transcript captions when `transcript.vtt` is ready
- A timestamped transcript list that seeks the video when clicked
- A delete button that removes the video row, generated database records, and stored objects

During processing, the backend also creates a browser-friendly `playback.mp4` with H.264/AAC when ffmpeg can transcode the upload. Existing videos without that derivative are repaired on demand the first time the player requests media.

Do not open `frontend/index.html` directly from the filesystem. The frontend should be served by the backend from `http://localhost:8080/`, otherwise browser fetches can fail.

### 5. Quick checks

Backend:

```bash
curl http://localhost:8080/healthz
```

Backend-to-LLM:

```bash
curl http://localhost:8080/api/llm/status
```

Videos:

```bash
curl http://localhost:8080/api/videos
```

Delete a video:

```bash
curl -X DELETE http://localhost:8080/api/videos/<video-id>
```

Upload:

```bash
curl -F "title=Demo video" -F "file=@sample.mp4" http://localhost:8080/api/videos/upload
```

Swagger UI:

```text
http://localhost:8080/swagger-ui
```

### 6. Stop everything

Stop the host backend with `Ctrl+C`.

Stop the Docker infrastructure:

```bash
docker compose stop postgres rustfs rustfs-init whisper
```

To remove local database, object storage, temp files, and Whisper model cache:

```bash
docker compose down -v
```

## Full Docker option

You can still run everything except llama inside Docker:

```bash
docker compose up --build
```

In that mode the backend container calls llama through:

```text
http://host.docker.internal:8100
```

For full Docker mode, start llama with `--host 0.0.0.0` so the backend container can reach it:

```bash
llama-server \
  -m ~/.cache/huggingface/hub/models--ggml-org--gemma-4-E4B-it-GGUF/snapshots/*/gemma-4-E4B-it-Q4_K_M.gguf \
  --alias "ggml-org/gemma-4-E4B-it-GGUF" \
  --host 0.0.0.0 \
  --port 8100 \
  --ctx-size 65536 \
  --reasoning-budget 512 \
  -t 8
```

If ports conflict, prefer the hybrid workflow above.

## Upload troubleshooting

If the frontend says `NetworkError when attempting to fetch resource`, check these first:

- Open the frontend from `http://localhost:8080/`, not from the local HTML file.
- Confirm the backend is running: `curl http://localhost:8080/healthz`.
- Confirm infrastructure is running: `docker compose ps`.
- Confirm MinIO is reachable: `curl http://localhost:9000/minio/health/live`.
- Try a small video first.

The backend allows uploads up to 1 GiB by default. Very large uploads still depend on browser memory, disk space, and available processing time.

## Default local credentials

Use these values for local Docker Compose runs.

| Service | URL | Username | Password | Notes |
|---|---|---|---|---|
| Backend API | http://localhost:8080 | N/A | N/A | Health endpoint: `/healthz` |
| Whisper API docs | http://localhost:8000/docs | N/A | N/A | OpenAPI UI for transcription endpoints |
| PostgreSQL | localhost:5432 | nexalearn | nexalearn | Database: `nexalearn` |
| Postgres UI (Adminer) | http://localhost:8081 | nexalearn | nexalearn | System: PostgreSQL, Server: `postgres`, Database: `nexalearn` |
| MinIO API | http://localhost:9000 | minio | minio12345 | S3-compatible API |
| MinIO Console | http://localhost:9001 | minio | minio12345 | Browser admin console |

MinIO bucket created by init service:
- `nexalearn`
