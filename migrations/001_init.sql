CREATE TABLE IF NOT EXISTS apps (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    start_url     TEXT NOT NULL,
    manifest_json TEXT NOT NULL DEFAULT '{}',
    created_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS permissions (
    app_id      TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    capability  TEXT NOT NULL,
    scope       TEXT NOT NULL DEFAULT '',
    granted_at  INTEGER NOT NULL,
    PRIMARY KEY (app_id, capability, scope)
);

CREATE TABLE IF NOT EXISTS sessions (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    app_id         TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    pid            INTEGER NOT NULL,
    started_at     INTEGER NOT NULL,
    ended_at       INTEGER,
    peak_mem_bytes INTEGER
);

CREATE INDEX IF NOT EXISTS idx_sessions_app ON sessions(app_id, started_at);