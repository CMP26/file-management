ALTER TABLE semantic_chat_cache
    ADD COLUMN IF NOT EXISTS question_terms TEXT[] NOT NULL DEFAULT '{}';

UPDATE semantic_chat_cache
SET question_terms = ARRAY(
    SELECT DISTINCT token
    FROM regexp_split_to_table(
        lower(semantic_chat_cache.question),
        '[^[:alnum:]+#]+'
    ) AS token
    WHERE length(token) > 1
      AND token <> ALL(ARRAY[
          'a', 'about', 'an', 'and', 'are', 'can', 'could', 'define', 'describe',
          'do', 'does', 'explain', 'for', 'give', 'how', 'i', 'in', 'is', 'it',
          'know', 'lesson', 'me', 'mean', 'of', 'on', 'please', 'tell', 'the',
          'this', 'to', 'video', 'what', 'which', 'who', 'why', 'would', 'you'
      ])
    ORDER BY token
)
WHERE cardinality(question_terms) = 0;

CREATE INDEX IF NOT EXISTS idx_semantic_chat_cache_question_terms
    ON semantic_chat_cache USING gin (question_terms);
