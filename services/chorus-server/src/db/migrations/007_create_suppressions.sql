CREATE TABLE suppressions (
    account_id  UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    channel     TEXT NOT NULL CHECK (channel IN ('sms', 'email')),
    recipient   TEXT NOT NULL,
    reason      TEXT NOT NULL CHECK (reason IN ('hard_bounce', 'manual')),
    source      TEXT NOT NULL CHECK (source IN ('chorus-mail', 'api')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, channel, recipient)
);
