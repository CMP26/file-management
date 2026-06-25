CREATE TABLE IF NOT EXISTS adaptive_exam_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL,
    video_id UUID NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
    ability_theta DOUBLE PRECISION NOT NULL DEFAULT 0,
    standard_error DOUBLE PRECISION NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'completed')),
    max_questions INTEGER NOT NULL DEFAULT 10,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS adaptive_exam_answers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    attempt_id UUID NOT NULL REFERENCES adaptive_exam_attempts(id) ON DELETE CASCADE,
    question_id UUID NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
    user_answer TEXT NOT NULL,
    is_correct BOOLEAN NOT NULL,
    score SMALLINT NOT NULL,
    ability_before DOUBLE PRECISION NOT NULL,
    ability_after DOUBLE PRECISION NOT NULL,
    item_difficulty DOUBLE PRECISION NOT NULL,
    answered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (attempt_id, question_id)
);

CREATE INDEX IF NOT EXISTS idx_adaptive_exam_attempts_user_started
    ON adaptive_exam_attempts (user_id, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_adaptive_exam_attempts_video_status
    ON adaptive_exam_attempts (video_id, status);

CREATE INDEX IF NOT EXISTS idx_adaptive_exam_answers_attempt
    ON adaptive_exam_answers (attempt_id, answered_at ASC);
