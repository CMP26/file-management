# NexaLearn local pipeline

This repository contains a local-first implementation of the upload, transcription, question generation, grading, and justification pipeline described in `nexalearn_pipeline_design.md`.

The default development stack is Docker Compose so the full system can be started with one command. The compose file wires together:

- PostgreSQL for structured data
- An S3-compatible object store exposed as `rustfs`
- A Whisper-compatible Python service for transcription
- The Rust Axum backend

The backend is configured to call your host-running llama.cpp server with Gemma on port `8100` using the OpenAI-compatible `/v1/chat/completions` API. Compose adds the Docker host gateway so the backend container can reach that endpoint directly.

The Whisper service is a real local transcription container using `faster-whisper`. It now accepts uploaded media, normalizes it through `ffmpeg`, and returns transcript text plus segments, which makes it usable with real video or audio files instead of only prebuilt WAV files.

## Start

```bash
docker compose up --build
```

If your llama.cpp endpoint listens somewhere else, set `GEMMA_BASE_URL` before running Compose.

Backend health check:

```bash
curl http://localhost:8080/healthz
```

Backend Swagger UI:

```bash
open http://localhost:8080/swagger-ui
```

OpenAPI JSON:

```bash
curl http://localhost:8080/api-docs/openapi.json
```

Upload endpoint:

```bash
curl -F "title=Demo video" -F "file=@sample.mp4" http://localhost:8080/api/videos/upload
```

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
