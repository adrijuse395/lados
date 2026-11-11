-- Lados Sync — D1 schema (SQLite).
-- Only metadata is stored here; vault contents live encrypted in R2.
--
-- Apply with:  wrangler d1 execute lados-sync --file=./schema.sql --remote

CREATE TABLE IF NOT EXISTS accounts (
  id          TEXT PRIMARY KEY,
  email       TEXT NOT NULL UNIQUE,
  salt        TEXT NOT NULL,
  verifier    TEXT NOT NULL,
  plan        TEXT NOT NULL DEFAULT 'active',  -- active | expired
  created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
  token_hash  TEXT PRIMARY KEY,
  account_id  TEXT NOT NULL,
  expires_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS vaults (
  id          TEXT PRIMARY KEY,
  account_id  TEXT NOT NULL,
  name        TEXT NOT NULL,
  version     INTEGER NOT NULL DEFAULT 0,
  updated_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_vaults_account  ON vaults(account_id);
CREATE INDEX IF NOT EXISTS idx_sessions_account ON sessions(account_id);
