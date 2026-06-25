ALTER TABLE adaptive_exam_attempts
    ADD COLUMN IF NOT EXISTS course_id UUID REFERENCES courses(id) ON DELETE CASCADE;

ALTER TABLE adaptive_exam_attempts
    ALTER COLUMN video_id DROP NOT NULL;

UPDATE adaptive_exam_attempts a
SET course_id = v.course_id
FROM videos v
WHERE a.video_id = v.id
  AND a.course_id IS NULL;

ALTER TABLE adaptive_exam_attempts
    DROP CONSTRAINT IF EXISTS adaptive_exam_attempts_scope_check;

ALTER TABLE adaptive_exam_attempts
    ADD CONSTRAINT adaptive_exam_attempts_scope_check
    CHECK (
        (video_id IS NOT NULL AND course_id IS NOT NULL)
        OR (video_id IS NULL AND course_id IS NOT NULL)
    );

CREATE INDEX IF NOT EXISTS idx_adaptive_exam_attempts_course_status
    ON adaptive_exam_attempts (course_id, status);
