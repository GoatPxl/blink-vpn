-- Backfills correlation ids on rows migration 0017 already copied without one.
--
-- 0017 shipped an INSERT that dropped correlation_id, so on any database that had already run it
-- every migrated provisioning row arrived NULL. The UI groups by correlation id, so those rows each
-- rendered as a separate one-step run — 69 entries where there were eight actual provisioning
-- attempts, with the failed step lost among them.
--
-- 0017 was corrected in place for databases that have not migrated yet. This exists because a
-- migration that has already run cannot be un-run, and editing history would leave anyone who
-- migrated in between silently broken.
--
-- Same rule as 0017: a provisioning run opens with `create.start`, so counting those per node
-- partitions the rows back into the runs they came from. Scoped to rows that are still NULL, so it
-- cannot disturb ids written by the live code path.

-- Computed into a temp table first, deliberately. Doing it as one self-referencing UPDATE would
-- have the run counter read `correlation_id IS NULL` on a column the same statement is busy
-- filling in, so the partitioning would shift under itself as rows were rewritten.
CREATE TEMP TABLE legacy_activity_runs AS
SELECT
  id,
  'legacy-' || node_id || '-' || SUM(CASE WHEN stage = 'create.start' THEN 1 ELSE 0 END)
    OVER (PARTITION BY node_id ORDER BY id ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
    AS run_id
FROM activity_events
WHERE correlation_id IS NULL
  AND category = 'provision'
  AND node_id IS NOT NULL;

UPDATE activity_events
SET correlation_id = (
  SELECT run_id FROM legacy_activity_runs WHERE legacy_activity_runs.id = activity_events.id
)
WHERE id IN (SELECT id FROM legacy_activity_runs);

DROP TABLE legacy_activity_runs;
