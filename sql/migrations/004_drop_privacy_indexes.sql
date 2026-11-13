-- Drop unused text-similarity privacy GIN indexes and helper functions (D84 / P.3).
-- Keep idx_events_share_granted for fast share.granted lookups.

DROP INDEX IF EXISTS idx_events_privacy_trgm;
DROP INDEX IF EXISTS idx_events_privacy_tsv;
DROP INDEX IF EXISTS idx_memories_privacy_trgm;
DROP INDEX IF EXISTS idx_memories_privacy_tsv;

DROP FUNCTION IF EXISTS privacy_clean_text(text);
DROP FUNCTION IF EXISTS event_line_text(text, jsonb);
