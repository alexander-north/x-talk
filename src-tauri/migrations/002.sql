ALTER TABLE audio_leases ADD COLUMN leases INTEGER NOT NULL DEFAULT 1;
INSERT INTO schema_migrations VALUES(2,strftime('%Y-%m-%dT%H:%M:%fZ','now'));
PRAGMA user_version=2;
