CREATE TABLE provider_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(id),
    channel TEXT NOT NULL CHECK (channel IN ('sms', 'email')),
    provider TEXT NOT NULL,
    priority INTEGER NOT NULL,
    credentials JSONB NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (account_id, channel, provider)
);

CREATE INDEX idx_provider_configs_account_channel
    ON provider_configs(account_id, channel);
