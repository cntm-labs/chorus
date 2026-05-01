-- services/chorus-server/src/db/migrations/008_create_idempotency_keys.sql
CREATE TABLE idempotency_keys (
    api_key_id        UUID NOT NULL REFERENCES api_keys(id) ON DELETE CASCADE,
    idempotency_key   TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 255),
    request_hash      BYTEA NOT NULL,
    request_method    TEXT NOT NULL,
    request_path      TEXT NOT NULL,
    status            TEXT NOT NULL CHECK (status IN ('in_progress', 'completed')),
    response_status   SMALLINT,
    response_body     BYTEA,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at        TIMESTAMPTZ NOT NULL DEFAULT (now() + interval '24 hours'),
    PRIMARY KEY (api_key_id, idempotency_key)
);

CREATE INDEX idempotency_keys_expires_at_idx ON idempotency_keys (expires_at);
