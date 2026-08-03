CREATE TABLE IF NOT EXISTS init_run_logs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  node_id TEXT NOT NULL,
  provider_slug TEXT NOT NULL,
  init_kind TEXT NOT NULL,
  stage TEXT NOT NULL,
  outcome TEXT NOT NULL,
  output TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_init_run_logs_node_created
  ON init_run_logs(node_id, created_at DESC);
