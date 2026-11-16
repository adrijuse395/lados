-- Lados Sync — D1 schema (SQLite).
-- Only metadata is stored here; vault contents live encrypted in R2.
--
-- Apply with:  wrangler d1 execute lados-sync --file=./schema.sql --remote

CREATE TABLE IF NOT EXISTS accounts (
  id            TEXT PRIMARY KEY,
  email         TEXT NOT NULL UNIQUE,
  salt          TEXT NOT NULL,
  verifier      TEXT NOT NULL,
  scheme        TEXT NOT NULL DEFAULT 'legacy',  -- legacy | argon2 | argon2v2 | opaque
  opaque_record TEXT,                            -- OPAQUE registration record
  password_changed_at INTEGER,                   -- when the master password last changed
  -- Entitlement (written only by the billing webhook). See docs/BILLING.md.
  plan          TEXT NOT NULL DEFAULT 'active',  -- comp|trialing|active|past_due|canceled|expired
  plan_valid_until       INTEGER,                -- end of the current paid period (ms)
  billing_provider       TEXT,                   -- 'paddle' | …
  billing_customer_id    TEXT,
  billing_subscription_id TEXT,
  trial_used_at          INTEGER,
  created_at    INTEGER NOT NULL
);

-- Billing webhook events already applied, keyed by the provider's event id (idempotency).
CREATE TABLE IF NOT EXISTS billing_events (
  id          TEXT PRIMARY KEY,
  received_at INTEGER NOT NULL
);

-- Emails granted complimentary (free, unlimited) access, whether or not the account exists yet:
-- on registration the plan becomes 'comp'; granting an existing account flips it immediately.
-- Managed with scripts/comp.mjs (requires the ADMIN_TOKEN Worker secret).
CREATE TABLE IF NOT EXISTS comp_grants (
  email      TEXT PRIMARY KEY,
  note       TEXT,
  created_at INTEGER NOT NULL
);

-- In-flight OPAQUE logins (short-lived handshake state).
CREATE TABLE IF NOT EXISTS logins (
  id           TEXT PRIMARY KEY,
  account_id   TEXT,
  server_state TEXT NOT NULL,
  expires_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
  token_hash  TEXT PRIMARY KEY,
  account_id  TEXT NOT NULL,
  expires_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS vaults (
  id          TEXT PRIMARY KEY,
  account_id  TEXT NOT NULL,
  name        TEXT NOT NULL,                   -- legacy plaintext label ('' for new vaults)
  name_cipher TEXT,                            -- encrypted label: the server cannot read it
  vault_uuid  TEXT,                            -- the vault's own id: its identity
  size        INTEGER NOT NULL DEFAULT 0,      -- current blob size, for the account quota
  version     INTEGER NOT NULL DEFAULT 0,
  updated_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_vaults_account  ON vaults(account_id);
CREATE INDEX IF NOT EXISTS idx_vaults_uuid     ON vaults(account_id, vault_uuid);

-- Fixed-window counters for abuse throttling (auth attempts, uploads).
CREATE TABLE IF NOT EXISTS rate_limits (
  bucket    TEXT PRIMARY KEY,
  count     INTEGER NOT NULL,
  reset_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_account ON sessions(account_id);
