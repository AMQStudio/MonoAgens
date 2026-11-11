-- Stability contract: every event carries the version of its payload schema.
-- Readers upcast older versions; the stored row is never rewritten (core.md §3).

ALTER TABLE events ADD COLUMN IF NOT EXISTS schema_version integer NOT NULL DEFAULT 1;

-- The version is part of the stored event: hard forgetting may scrub the payload only,
-- so the immutability trigger now compares schema_version too.
CREATE OR REPLACE FUNCTION events_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE'
       AND current_setting('monoagens.forgetting', true) = 'on'
       AND NEW.payload = '{"forgotten": true}'::jsonb
       AND (NEW.id, NEW.ts, NEW.kind, NEW.who, NEW.place, NEW.body, NEW.caused_by,
            NEW.origin_actor, NEW.schema_version)
           IS NOT DISTINCT FROM
           (OLD.id, OLD.ts, OLD.kind, OLD.who, OLD.place, OLD.body, OLD.caused_by,
            OLD.origin_actor, OLD.schema_version) THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'events table is append-only';
END;
$$;
