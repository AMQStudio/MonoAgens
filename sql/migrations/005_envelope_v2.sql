-- Contract v2 envelope (contract.md §3): the fields the core reads, plus the two opaque
-- mappings the middleware and the adapters own. Stored events are never rewritten: older rows
-- keep schema_version 1 and are upcast on read.

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

-- Recall searches the description, never the payload.
CREATE INDEX IF NOT EXISTS idx_events_description_trgm
    ON events USING gin (description gin_trgm_ops);

-- The Log stays immutable. Hard forgetting may scrub the three content fields (the payload,
-- the description and the numbers computed from it) and nothing else.
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
