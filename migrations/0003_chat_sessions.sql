ALTER TABLE chat_conversations
    DROP CONSTRAINT IF EXISTS chat_conversations_user_id_video_id_key;

ALTER TABLE chat_conversations
    ADD COLUMN IF NOT EXISTS name TEXT NOT NULL DEFAULT 'Untitled chat';
