CREATE INDEX entries_audio ON entries(json_extract(document,'$.audio')) WHERE json_extract(document,'$.audio') IS NOT NULL;
INSERT INTO schema_migrations VALUES(3,strftime('%Y-%m-%dT%H:%M:%fZ','now'));
PRAGMA user_version=3;
