-- MonoAgens schema: the four books.
-- Idempotent: safe to apply repeatedly (scripts/db_init).

CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- events: the Log (append-only, the source of truth).
CREATE TABLE IF NOT EXISTS events (
    id        bigserial PRIMARY KEY,
    ts        timestamptz NOT NULL DEFAULT now(),
    occurred_at timestamptz,
    kind      text NOT NULL,
    description text NOT NULL DEFAULT '',
    who       text,
    place     text,
    body      text,
    caused_by bigint REFERENCES events (id),
    origin_actor text,
    features  jsonb NOT NULL DEFAULT '{}'::jsonb,
    backfill  boolean NOT NULL DEFAULT false,
    imported  boolean NOT NULL DEFAULT false,
    payload   jsonb NOT NULL DEFAULT '{}'::jsonb,
    labels    jsonb NOT NULL DEFAULT '{}'::jsonb,
    schema_version integer NOT NULL DEFAULT 1
);

ALTER TABLE events ADD COLUMN IF NOT EXISTS origin_actor text;
ALTER TABLE events ADD COLUMN IF NOT EXISTS schema_version integer NOT NULL DEFAULT 1;
ALTER TABLE events ADD COLUMN IF NOT EXISTS occurred_at timestamptz;
ALTER TABLE events ADD COLUMN IF NOT EXISTS description text NOT NULL DEFAULT '';
ALTER TABLE events ADD COLUMN IF NOT EXISTS features jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE events ADD COLUMN IF NOT EXISTS backfill boolean NOT NULL DEFAULT false;
ALTER TABLE events ADD COLUMN IF NOT EXISTS imported boolean NOT NULL DEFAULT false;
ALTER TABLE events ADD COLUMN IF NOT EXISTS labels jsonb NOT NULL DEFAULT '{}'::jsonb;
UPDATE events SET occurred_at = ts WHERE occurred_at IS NULL;
ALTER TABLE events ALTER COLUMN occurred_at SET NOT NULL;

-- A row written without a time of its own happened when it was recorded (contract.md §3).
CREATE OR REPLACE FUNCTION events_default_occurred_at() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.occurred_at IS NULL THEN
        NEW.occurred_at := NEW.ts;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS events_set_occurred_at ON events;
CREATE TRIGGER events_set_occurred_at
    BEFORE INSERT ON events
    FOR EACH ROW
    EXECUTE FUNCTION events_default_occurred_at();

CREATE INDEX IF NOT EXISTS idx_events_place_id ON events (place, id);
CREATE INDEX IF NOT EXISTS idx_events_description_trgm
    ON events USING gin (description gin_trgm_ops);

-- The Log is immutable: reject INSERT-after-the-fact rewrites, UPDATE and DELETE.
-- The one UPDATE exception is hard forgetting: a dedicated function sets
-- monoagens.forgetting and scrubs the payload, the description and the features only.
CREATE OR REPLACE FUNCTION events_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE'
       AND current_setting('monoagens.forgetting', true) = 'on'
       AND NEW.payload = '{"forgotten": true}'::jsonb
       AND NEW.description = ''
       AND NEW.features = '{}'::jsonb
       AND (NEW.id, NEW.ts, NEW.kind, NEW.who, NEW.place, NEW.body, NEW.caused_by,
            NEW.origin_actor, NEW.schema_version, NEW.occurred_at, NEW.backfill, NEW.imported,
            NEW.labels)
           IS NOT DISTINCT FROM
           (OLD.id, OLD.ts, OLD.kind, OLD.who, OLD.place, OLD.body, OLD.caused_by,
            OLD.origin_actor, OLD.schema_version, OLD.occurred_at, OLD.backfill, OLD.imported,
            OLD.labels) THEN
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
-- kind: promise, wait, predict or guard (core.md §5).
CREATE TABLE IF NOT EXISTS expectations (
    id            bigserial PRIMARY KEY,
    kind          text NOT NULL DEFAULT 'promise'
                  CHECK (kind IN ('promise', 'wait', 'predict', 'guard')),
    who           text,
    place         text,
    about         text NOT NULL,
    cue_time      jsonb,
    cue_event     jsonb,
    condition     jsonb,
    condition_text text,
    condition_vector vector(384),
    intent        text,
    weight        real NOT NULL DEFAULT 0.5 CHECK (weight >= 0.0 AND weight <= 1.0),
    policy        jsonb NOT NULL DEFAULT '{}'::jsonb,
    status        text NOT NULL DEFAULT 'active',
    next_check_at timestamptz,
    last_fired_at timestamptz,
    fire_count    integer NOT NULL DEFAULT 0,
    source_event  bigint REFERENCES events (id),
    derived_from  bigint[] NOT NULL DEFAULT '{}',
    origin_actor  text,
    expires_at    timestamptz NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    origin        text NOT NULL DEFAULT 'pfc'
);

ALTER TABLE expectations ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'pfc';
ALTER TABLE expectations
    ADD COLUMN IF NOT EXISTS derived_from bigint[] NOT NULL DEFAULT '{}';
ALTER TABLE expectations ADD COLUMN IF NOT EXISTS origin_actor text;
ALTER TABLE expectations ADD COLUMN IF NOT EXISTS condition jsonb;
ALTER TABLE expectations ADD COLUMN IF NOT EXISTS condition_text text;
ALTER TABLE expectations ADD COLUMN IF NOT EXISTS condition_vector vector(384);
ALTER TABLE expectations
    ADD COLUMN IF NOT EXISTS kind text NOT NULL DEFAULT 'promise';

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                   WHERE conname = 'expectations_kind_check'
                     AND conrelid = 'expectations'::regclass) THEN
        ALTER TABLE expectations ADD CONSTRAINT expectations_kind_check
            CHECK (kind IN ('promise', 'wait', 'predict', 'guard'));
    END IF;
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_schema = current_schema() AND table_name = 'expectations'
                 AND column_name = 'expect') THEN
        EXECUTE 'UPDATE expectations SET condition = COALESCE(condition, expect)';
        EXECUTE 'ALTER TABLE expectations DROP COLUMN expect';
    END IF;
END;
$$;

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
