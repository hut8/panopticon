CREATE TABLE sentinels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL DEFAULT 'sentinel',
    secret TEXT NOT NULL UNIQUE,
    connected BOOLEAN NOT NULL DEFAULT FALSE,
    last_connected_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE sentinel_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sentinel_id UUID NOT NULL REFERENCES sentinels(id),
    message TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_sentinel_logs_sentinel_id_created ON sentinel_logs (sentinel_id, created_at DESC);
