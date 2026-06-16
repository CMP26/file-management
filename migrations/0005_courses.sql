CREATE TABLE IF NOT EXISTS courses (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL UNIQUE,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO courses (title, description)
VALUES ('Default Course', 'Backfilled course for videos created before course support.')
ON CONFLICT (title) DO NOTHING;

ALTER TABLE videos
    ADD COLUMN IF NOT EXISTS course_id UUID REFERENCES courses(id);

UPDATE videos
SET course_id = (SELECT id FROM courses WHERE title = 'Default Course' LIMIT 1)
WHERE course_id IS NULL;

ALTER TABLE videos
    ALTER COLUMN course_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_videos_course_id ON videos(course_id);
CREATE INDEX IF NOT EXISTS idx_questions_video_id ON questions(video_id);
