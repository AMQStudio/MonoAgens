-- Indexes and immutable text extractors for fast SQL privacy candidate lookup (B.3, D45, D80).

CREATE OR REPLACE FUNCTION event_line_text(kind text, payload jsonb) RETURNS text
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE AS $$
SELECT CASE
    WHEN kind = 'act.out' THEN coalesce(payload->'arguments'->>'text', '')
    WHEN kind = 'act.result' THEN coalesce(
        NULLIF(payload->'result'->>'description', ''),
        NULLIF(payload->'result'->>'text', ''),
        NULLIF(payload->'result'->>'reason', ''),
        NULLIF(payload->'result'->>'about', ''),
        NULLIF(payload->'result'->'arguments'->>'text', ''),
        ''
    )
    ELSE coalesce(
        NULLIF(payload->>'description', ''),
        NULLIF(payload->>'text', ''),
        NULLIF(payload->>'reason', ''),
        NULLIF(payload->>'about', ''),
        NULLIF(payload->'arguments'->>'text', ''),
        ''
    )
END;
$$;

CREATE OR REPLACE FUNCTION privacy_clean_text(txt text) RETURNS text
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE AS $$
SELECT regexp_replace(txt, '[^0-9a-zA-ZÀ-ÿ_]+', ' ', 'g');
$$;

CREATE INDEX IF NOT EXISTS idx_events_privacy_trgm
    ON events USING gin (event_line_text(kind, payload) gin_trgm_ops)
    WHERE kind IN ('percept', 'act.out', 'act.result') AND place IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_events_privacy_tsv
    ON events USING gin (to_tsvector('simple', privacy_clean_text(event_line_text(kind, payload))))
    WHERE kind IN ('percept', 'act.out', 'act.result') AND place IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_events_share_granted
    ON events (id)
    WHERE kind = 'share.granted';

CREATE INDEX IF NOT EXISTS idx_memories_privacy_trgm
    ON memories USING gin (text gin_trgm_ops)
    WHERE valid_to IS NULL;

CREATE INDEX IF NOT EXISTS idx_memories_privacy_tsv
    ON memories USING gin (to_tsvector('simple', privacy_clean_text(text)))
    WHERE valid_to IS NULL;
