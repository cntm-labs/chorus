-- services/chorus-server/src/db/migrations/010_create_totp.sql
CREATE TABLE totp_users (
    account_id        UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    user_id           TEXT NOT NULL CHECK (length(user_id) BETWEEN 1 AND 255),
    secret            BYTEA NOT NULL,
    status            TEXT  NOT NULL CHECK (status IN ('pending','active','disabled')),
    algorithm         TEXT  NOT NULL DEFAULT 'SHA1',
    digits            SMALLINT NOT NULL DEFAULT 6,
    period_secs       SMALLINT NOT NULL DEFAULT 30,
    issuer            TEXT,
    label             TEXT,
    last_verified_at  TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    activated_at      TIMESTAMPTZ,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, user_id)
);

CREATE INDEX totp_users_account_status_idx ON totp_users (account_id, status);

CREATE TABLE totp_backup_codes (
    id          BIGSERIAL PRIMARY KEY,
    account_id  UUID NOT NULL,
    user_id     TEXT NOT NULL,
    code_hash   BYTEA NOT NULL,
    used_at     TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (account_id, user_id)
        REFERENCES totp_users (account_id, user_id)
        ON DELETE CASCADE
);

CREATE INDEX totp_backup_codes_user_idx ON totp_backup_codes (account_id, user_id, used_at);
CREATE UNIQUE INDEX totp_backup_codes_hash_uniq ON totp_backup_codes (account_id, user_id, code_hash);
