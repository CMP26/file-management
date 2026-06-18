CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS semantic_chat_cache (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    video_id UUID NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
    embedding_model TEXT NOT NULL,
    question TEXT NOT NULL,
    embedding vector(768) NOT NULL,
    answer TEXT NOT NULL,
    sources_json TEXT,
    hit_count BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_hit_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_semantic_chat_cache_video_model
    ON semantic_chat_cache (video_id, embedding_model);

CREATE INDEX IF NOT EXISTS idx_semantic_chat_cache_embedding_cosine
    ON semantic_chat_cache USING hnsw (embedding vector_cosine_ops);
