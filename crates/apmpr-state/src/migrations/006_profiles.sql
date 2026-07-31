CREATE TABLE imported_profiles (
    name TEXT PRIMARY KEY,
    source_name TEXT NOT NULL,
    source_commit TEXT,
    content_hash TEXT NOT NULL,
    definition_json TEXT NOT NULL,
    imported_at INTEGER NOT NULL
);
