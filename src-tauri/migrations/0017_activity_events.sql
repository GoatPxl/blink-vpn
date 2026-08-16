-- Replaces init_run_logs with a general lifecycle activity log.
--
-- init_run_logs was written by exactly one flow (create_vpn_node_to_ready), so connect, disconnect,
-- destroy and force-cleanup left no record at all and the app forgot every session on exit. Its
-- shape blocked the fix rather than merely lacking it:
--
--   * node_id was NOT NULL, so an app-level event (a startup reconcile) had nowhere to go.
--   * init_kind held "cloud-init" / "docker-init", chosen by a provider-slug comparison in the
--     orchestrator — a provider branch outside an adapter, which AGENTS.md 3 forbids. provider_slug
--     already carries that distinction, so the column is dropped rather than carried forward.
--
-- Existing rows migrate as category 'provision'; nothing is lost that was not derivable.

CREATE TABLE IF NOT EXISTS activity_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  -- NULL for events that belong to the app rather than to one node.
  node_id TEXT NULL,
  provider_slug TEXT NULL,
  -- provision | connect | disconnect | destroy | cleanup | app
  category TEXT NOT NULL,
  stage TEXT NOT NULL,
  -- info | success | error
  outcome TEXT NOT NULL,
  -- Groups the stages of one run so the UI can collapse them into a single entry.
  correlation_id TEXT NULL,
  -- Redacted at the write boundary, never on read.
  detail TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_activity_events_created ON activity_events(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_activity_events_node ON activity_events(node_id, created_at DESC);

-- Legacy rows have no correlation id, so one is synthesised: a provisioning run always opens with
-- `create.start`, and counting those per node partitions the history into the runs it came from.
-- Without this every migrated row renders as its own single-step run, which buries a failure among
-- the seven successful steps that preceded it — exactly what the grouping exists to prevent.
INSERT INTO activity_events
    (node_id, provider_slug, category, stage, outcome, correlation_id, detail, created_at)
  SELECT
    node_id,
    provider_slug,
    'provision',
    stage,
    outcome,
    'legacy-' || node_id || '-' || SUM(CASE WHEN stage = 'create.start' THEN 1 ELSE 0 END)
      OVER (PARTITION BY node_id ORDER BY id ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW),
    output,
    created_at
  FROM init_run_logs;

DROP TABLE IF EXISTS init_run_logs;
