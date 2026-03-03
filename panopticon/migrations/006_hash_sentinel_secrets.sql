CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- Replace plaintext secrets with SHA-256 hashes
ALTER TABLE sentinels RENAME COLUMN secret TO secret_hash;
UPDATE sentinels SET secret_hash = encode(digest(secret_hash::bytea, 'sha256'), 'hex');
