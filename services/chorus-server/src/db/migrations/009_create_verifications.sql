-- services/chorus-server/src/db/migrations/009_create_verifications.sql
CREATE TABLE verifications (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    api_key_id      UUID NOT NULL REFERENCES api_keys(id) ON DELETE CASCADE,
    channel         TEXT NOT NULL CHECK (channel IN ('sms', 'email')),
    recipient       TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'canceled', 'expired')),
    check_attempts  INTEGER NOT NULL DEFAULT 0,
    resend_attempts INTEGER NOT NULL DEFAULT 0,
    cost_micro      BIGINT  NOT NULL DEFAULT 0,
    cost_currency   TEXT    NOT NULL DEFAULT 'USD',
    environment     TEXT    NOT NULL,
    app_name        TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL
);

CREATE INDEX verifications_account_created_idx ON verifications (account_id, created_at DESC);
CREATE INDEX verifications_recipient_idx       ON verifications (recipient, created_at DESC);
CREATE INDEX verifications_pending_expiry_idx  ON verifications (expires_at)
    WHERE status = 'pending';
