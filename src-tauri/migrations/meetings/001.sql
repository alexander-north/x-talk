CREATE TABLE meetings (
    id TEXT PRIMARY KEY,
    added_at TEXT NOT NULL,
    revision INTEGER NOT NULL,
    document TEXT NOT NULL
);
CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK(id=1), retain_count INTEGER);
INSERT INTO settings VALUES (1, 50);
CREATE TABLE audio_leases (
    id TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
    count INTEGER NOT NULL
);
-- Deletion intent survives filesystem failures and restarts.
CREATE TABLE file_gc (id TEXT PRIMARY KEY);
PRAGMA user_version=1;
