CREATE TABLE IF NOT EXISTS system_config (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
INSERT INTO system_config (key, value) VALUES ('sentinel_mode', 'guard');

CREATE TABLE IF NOT EXISTS access_cards (
    id         UUID PRIMARY KEY NOT NULL DEFAULT gen_random_uuid(),
    tag_id     TEXT UNIQUE NOT NULL,   -- hex like "80:00:48:23:4C"
    label      TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS scan_log (
    id         UUID PRIMARY KEY NOT NULL DEFAULT gen_random_uuid(),
    tag_id     TEXT NOT NULL,
    action     TEXT NOT NULL,           -- 'granted', 'denied', 'enrolled'
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
