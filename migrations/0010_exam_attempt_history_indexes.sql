CREATE INDEX IF NOT EXISTS idx_exam_attempts_user_started
    ON exam_attempts(user_id, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_exam_attempts_user_video_started
    ON exam_attempts(user_id, video_id, started_at DESC);
