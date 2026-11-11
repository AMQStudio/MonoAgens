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
    caused_by bigint REFERENCES events (id),
    origin_actor text
);

ALTER TABLE events ADD COLUMN IF NOT EXISTS origin_actor text;

CREATE INDEX IF NOT EXISTS idx_events_place_id ON events (place, id);

-- The Log is immutable: reject INSERT-after-the-fact rewrites, UPDATE and DELETE.
-- The one UPDATE exception is hard forgetting: a dedicated function sets
-- monoagens.forgetting and replaces the payload with {"forgotten": true} only.
CREATE OR REPLACE FUNCTION events_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE'
       AND current_setting('monoagens.forgetting', true) = 'on'
       AND NEW.payload = '{"forgotten": true}'::jsonb
       AND (NEW.id, NEW.ts, NEW.kind, NEW.who, NEW.place, NEW.body, NEW.caused_by,
            NEW.origin_actor)
           IS NOT DISTINCT FROM
           (OLD.id, OLD.ts, OLD.kind, OLD.who, OLD.place, OLD.body, OLD.caused_by,
            OLD.origin_actor) THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'events table is append-only';
END;
$$;

DROP TRIGGER IF EXISTS events_no_update_delete ON events;
CREATE TRIGGER events_no_update_delete
    BEFORE UPDATE OR DELETE ON events
    FOR EACH ROW
    EXECUTE FUNCTION events_immutable();

-- event_vectors: one embedding per event, written once. Rows leave only by fading
-- or hard forgetting, each through one dedicated path that writes an event.
CREATE TABLE IF NOT EXISTS event_vectors (
    event_id     bigint PRIMARY KEY REFERENCES events (id),
    embedding    vector(384) NOT NULL,
    derived_from bigint[] NOT NULL DEFAULT '{}'
);

ALTER TABLE event_vectors ADD COLUMN IF NOT EXISTS derived_from bigint[] NOT NULL DEFAULT '{}';

CREATE OR REPLACE FUNCTION event_vectors_write_once() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'event vectors are write-once';
END;
$$;

CREATE OR REPLACE FUNCTION event_vectors_guarded_delete() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('monoagens.allowed_vector_delete', true) = 'on' THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'event vectors leave only by fading or hard forgetting';
END;
$$;

DROP TRIGGER IF EXISTS event_vectors_no_update_delete ON event_vectors;
DROP TRIGGER IF EXISTS event_vectors_no_update ON event_vectors;
CREATE TRIGGER event_vectors_no_update
    BEFORE UPDATE ON event_vectors
    FOR EACH ROW
    EXECUTE FUNCTION event_vectors_write_once();

DROP TRIGGER IF EXISTS event_vectors_guarded_delete ON event_vectors;
CREATE TRIGGER event_vectors_guarded_delete
    BEFORE DELETE ON event_vectors
    FOR EACH ROW
    EXECUTE FUNCTION event_vectors_guarded_delete();

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
    derived_from  bigint[] NOT NULL DEFAULT '{}',
    expires_at    timestamptz NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    origin        text NOT NULL DEFAULT 'pfc'
);

ALTER TABLE expectations ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'pfc';
ALTER TABLE expectations
    ADD COLUMN IF NOT EXISTS derived_from bigint[] NOT NULL DEFAULT '{}';

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
    private          boolean NOT NULL DEFAULT false,
    valid_from       timestamptz NOT NULL DEFAULT now(),
    valid_to         timestamptz,
    superseded_by    bigint REFERENCES memories (id),
    derived_from     bigint[] NOT NULL DEFAULT '{}',
    created_at       timestamptz NOT NULL DEFAULT now(),
    last_accessed_at timestamptz NOT NULL DEFAULT now(),
    use_count        integer NOT NULL DEFAULT 0
);

ALTER TABLE memories ADD COLUMN IF NOT EXISTS use_count integer NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS private boolean NOT NULL DEFAULT false;

-- Lineage was named source_events before T4; keep old databases idempotent.
ALTER TABLE memories ADD COLUMN IF NOT EXISTS derived_from bigint[] NOT NULL DEFAULT '{}';
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'memories' AND column_name = 'source_events') THEN
        EXECUTE 'UPDATE memories SET derived_from = derived_from || source_events';
        EXECUTE 'ALTER TABLE memories DROP COLUMN source_events';
    END IF;
END;
$$;
