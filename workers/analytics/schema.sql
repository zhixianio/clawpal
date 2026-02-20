CREATE TABLE IF NOT EXISTS pings (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  install_id TEXT NOT NULL,
  version TEXT NOT NULL,
  platform TEXT NOT NULL,
  date TEXT NOT NULL DEFAULT (date('now')),
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_pings_date ON pings(date);
CREATE INDEX IF NOT EXISTS idx_pings_install_id ON pings(install_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_pings_daily ON pings(install_id, date);
