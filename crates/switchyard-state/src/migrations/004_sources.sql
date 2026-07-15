CREATE TABLE registered_sources (
    name TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('managed', 'unmanaged')),
    path TEXT NOT NULL UNIQUE,
    repository_path TEXT,
    requested_ref TEXT,
    created_at INTEGER NOT NULL,
    managed_relative_path TEXT,
    CHECK (
        (kind = 'managed' AND managed_relative_path IS NOT NULL) OR
        (kind = 'unmanaged' AND managed_relative_path IS NULL)
    )
);
