#!/usr/bin/env bash
set -euo pipefail

export DATABASE_URL="${DATABASE_URL:-postgres://nexalearn:nexalearn@localhost:5432/nexalearn}"
export RUSTFS_ENDPOINT="${RUSTFS_ENDPOINT:-http://localhost:9000}"
export RUSTFS_BUCKET="${RUSTFS_BUCKET:-nexalearn}"
export RUSTFS_ACCESS_KEY="${RUSTFS_ACCESS_KEY:-minio}"
export RUSTFS_SECRET_KEY="${RUSTFS_SECRET_KEY:-minio12345}"
export AWS_ACCESS_KEY_ID="${AWS_ACCESS_KEY_ID:-$RUSTFS_ACCESS_KEY}"
export AWS_SECRET_ACCESS_KEY="${AWS_SECRET_ACCESS_KEY:-$RUSTFS_SECRET_KEY}"
export AWS_REGION="${AWS_REGION:-us-east-1}"
export GEMMA_BASE_URL="${GEMMA_BASE_URL:-http://localhost:8100}"
export GEMMA_MODEL="${GEMMA_MODEL:-ggml-org/gemma-4-E4B-it-GGUF}"
export OLLAMA_BASE_URL="${OLLAMA_BASE_URL:-http://localhost:11434}"
export EMBEDDING_MODEL="${EMBEDDING_MODEL:-nomic-embed-text}"
export SEMANTIC_CACHE_THRESHOLD="${SEMANTIC_CACHE_THRESHOLD:-0.70}"
export WHISPER_URL="${WHISPER_URL:-http://localhost:8000}"
export TMP_DIR="${TMP_DIR:-/tmp/nexalearn}"
export BIND_ADDR="${BIND_ADDR:-127.0.0.1:8080}"

mkdir -p "$TMP_DIR"

echo "Building React frontend..."
if [[ ! -d frontend/node_modules ]]; then
  (cd frontend && npm ci)
fi
(cd frontend && npm run build)

echo "Starting NexaLearn backend on http://$BIND_ADDR"
echo "LLM: $GEMMA_BASE_URL ($GEMMA_MODEL)"
echo "Embeddings: $OLLAMA_BASE_URL ($EMBEDDING_MODEL, threshold $SEMANTIC_CACHE_THRESHOLD)"
echo "Postgres: $DATABASE_URL"
echo "RustFS: $RUSTFS_ENDPOINT/$RUSTFS_BUCKET"
echo "Whisper: $WHISPER_URL"

cargo run
