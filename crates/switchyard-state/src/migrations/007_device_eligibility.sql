ALTER TABLE devices RENAME TO devices_ssh_status;

CREATE TABLE devices (
    name TEXT PRIMARY KEY,
    host TEXT NOT NULL,
    port INTEGER NOT NULL DEFAULT 22 CHECK(port BETWEEN 1 AND 65535),
    user TEXT NOT NULL,
    identity_file TEXT,
    created_at INTEGER NOT NULL,
    last_checked_at INTEGER,
    last_check_status TEXT NOT NULL DEFAULT 'never'
        CHECK(last_check_status IN (
            'never', 'ok', 'eligible', 'ineligible', 'unreachable', 'auth-failed'
        )),
    last_check_detail TEXT
);

INSERT INTO devices(
    name, host, port, user, identity_file, created_at, last_checked_at,
    last_check_status, last_check_detail
)
SELECT
    name, host, port, user, identity_file, created_at, last_checked_at,
    last_check_status, last_check_detail
FROM devices_ssh_status;

DROP TABLE devices_ssh_status;
