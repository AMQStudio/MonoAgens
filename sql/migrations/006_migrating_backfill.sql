-- A migration may fill one row's new column, and only while it says so.
--
-- The Log is append-only, so the trigger refuses every UPDATE. Two escapes exist and both are
-- narrow: hard forgetting (the payload and the fields computed from it) and a migration that
-- adds a column whose value it can compute from the row itself — `occurred_at` from `ts`, here.
-- Anything else still raises, so a bug cannot rewrite a fact the Log already stated.

CREATE OR REPLACE FUNCTION events_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE'
       AND current_setting('monoagens.migrating', true) = 'on'
       AND NEW.occurred_at IS NOT DISTINCT FROM OLD.ts
       AND (NEW.id, NEW.ts, NEW.kind, NEW.who, NEW.place, NEW.body, NEW.caused_by,
            NEW.origin_actor, NEW.schema_version, NEW.backfill, NEW.imported, NEW.labels)
           IS NOT DISTINCT FROM
           (OLD.id, OLD.ts, OLD.kind, OLD.who, OLD.place, OLD.body, OLD.caused_by,
            OLD.origin_actor, OLD.schema_version, OLD.backfill, OLD.imported, OLD.labels) THEN
        RETURN NEW;
    END IF;
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
