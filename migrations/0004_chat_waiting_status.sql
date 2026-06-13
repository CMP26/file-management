ALTER TABLE chat_conversations
    ADD COLUMN IF NOT EXISTS is_waiting BOOLEAN NOT NULL DEFAULT false;
