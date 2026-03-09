CREATE TABLE IF NOT EXISTS lock_state_log (
    id         UUID PRIMARY KEY NOT NULL DEFAULT gen_random_uuid(),
    device_id  TEXT NOT NULL,
    lock_state TEXT NOT NULL,              -- 'locked', 'unlocked'
    source     TEXT NOT NULL,              -- 'webhook', 'api', 'api_deferred'
    user_id    UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_lock_state_log_device_id ON lock_state_log (device_id);
CREATE INDEX idx_lock_state_log_created_at ON lock_state_log (created_at);
