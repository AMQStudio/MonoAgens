-- MonoAgens schema: the four books.
-- Idempotent: safe to apply repeatedly (scripts/db_init).

CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- events: the Log (append-only, the source of truth).
CREATE TABLE IF NOT EXISTS events (
    id        bigserial PRIMARY KEY,
    ts        timestamptz NOT NULL DEFAULT now(),
    kind      text NOT NULL,
    who       text,
    place     text,
    body      text,
    payload   jsonb NOT NULL DEFAULT '{}'::jsonb,
    caused_by bigint REFERENCES events (id)
);

-- The Log is immutable: reject UPDATE and DELETE.
CREATE OR REPLACE FUNCTION events_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'events table is append-only';
END;
$$;

DROP TRIGGER IF EXISTS events_no_update_delete ON events;
CREATE TRIGGER events_no_update_delete
    BEFORE UPDATE OR DELETE ON events
    FOR EACH ROW
    EXECUTE FUNCTION events_immutable();

-- state: the Status Board (a projection; may be dropped and rebuilt).
CREATE TABLE IF NOT EXISTS state (
    key             text PRIMARY KEY,
    value           jsonb NOT NULL,
    updated_by_event bigint NOT NULL REFERENCES events (id)
);

-- expectations: the Expectation Book, one primitive for every scenario.
CREATE TABLE IF NOT EXISTS expectations (
    id            bigserial PRIMARY KEY,
    who           text,
    place         text,
    about         text NOT NULL,
    cue_time      jsonb,
    cue_event     jsonb,
    expect        jsonb NOT NULL,
    intent        text,
    weight        real NOT NULL DEFAULT 0.5 CHECK (weight >= 0.0 AND weight <= 1.0),
    policy        jsonb NOT NULL DEFAULT '{}'::jsonb,
    status        text NOT NULL DEFAULT 'active',
    next_check_at timestamptz,
    last_fired_at timestamptz,
    fire_count    integer NOT NULL DEFAULT 0,
    source_event  bigint REFERENCES events (id),
    expires_at    timestamptz NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_expectations_next_check_at
    ON expectations (next_check_at);

-- memories: the Memory Book (produced by sleep).
CREATE TABLE IF NOT EXISTS memories (
    id               bigserial PRIMARY KEY,
    who              text,
    place            text,
    text             text NOT NULL,
    embedding        vector(384),
    tsv              tsvector GENERATED ALWAYS AS (to_tsvector('simple', text)) STORED,
    importance       real NOT NULL DEFAULT 0.5,
    valid_from       timestamptz NOT NULL DEFAULT now(),
    valid_to         timestamptz,
    superseded_by    bigint REFERENCES memories (id),
    source_events    bigint[] NOT NULL DEFAULT '{}',
    created_at       timestamptz NOT NULL DEFAULT now(),
    last_accessed_at timestamptz NOT NULL DEFAULT now()
);
