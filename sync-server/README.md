# Lados Sync server

The **zero-knowledge** sync backend for the [Lados](https://github.com/ladospwm/lados)
password manager, running as a **Cloudflare Worker**.

It stores an **opaque, encrypted** copy of each vault (in R2) plus minimal metadata
(accounts, versions) in D1. It **never** sees plaintext and never resolves conflicts —
all merging happens on the user's device.

> The matching client lives in the main Lados repo, behind the `sync` Cargo feature.
> Design: `docs/SYNC.md` in that repo.

## Endpoints

| Method | Path | Purpose |
| --- | --- | --- |
| POST | `/auth/register/start` | `{ email, registrationRequest }` → `{ registrationResponse }` |
| POST | `/auth/register/finish` | `{ email, registrationRecord }` → `{ token, plan }` |
| POST | `/auth/login/start` | `{ email, startLoginRequest }` → `{ loginId, loginResponse }` |
| POST | `/auth/login/finish` | `{ loginId, finishLoginRequest }` → `{ token, plan }` |
| POST | `/auth/upgrade/start` / `finish` | attach an OPAQUE record to a pre-OPAQUE account |
| POST | `/auth/login` | legacy fast-verifier login, kept only to upgrade old accounts |
| GET | `/me` | account + plan |
| GET | `/vaults` | list vaults |
| POST | `/vaults` | `{ vaultUuid, nameCipher }` → create a vault (name stored encrypted) |
| GET | `/vaults/:id` | `{ version, blob }` (base64) |
| PUT | `/vaults/:id` | raw encrypted bytes + `If-Match: <version>` → `{ version }` (409 on conflict) |
| DELETE | `/vaults/:id` | remove a vault (and its blobs) |
| DELETE | `/sessions` | revoke every other session ("sign out everywhere") |
| DELETE | `/account` | delete the account, its data and its subscription |
| POST | `/billing/checkout` · `/cancel` · `/resume` | Paddle checkout, cancel at period end, undo it |

Auth is a bearer token (opaque, stored hashed, 30-day TTL).

Accounts are **OPAQUE** (aPAKE): the client proves knowledge of the master password without ever
sending it, and the server stores only an OPAQUE registration record. The old fast-verifier
`/auth/register` endpoint is **removed**; `/auth/login` and `/auth/upgrade/*` remain only so
accounts created before OPAQUE can sign in once and be upgraded.

Requests are throttled per IP, per email and per account; exceeding a limit returns `429`.

### Secrets

| Name | What |
| --- | --- |
| `OPAQUE_SERVER_SETUP` | OPAQUE server setup (OPRF seed + key pair). **Critical.** |
| `PADDLE_API_KEY` | Paddle API key (server-side checkout/cancel/resume). |
| `PADDLE_WEBHOOK_SECRET` | Webhook destination secret; verifies entitlement events. |
| `ADMIN_TOKEN` | *Optional.* Enables the `/admin/comp` routes (see below). |

Set them with `wrangler secret put <name>`, or in the dashboard with type **Secret** — never as
plaintext variables, because a `wrangler deploy` replaces the script's variables with
`wrangler.toml` and drops dashboard-only ones. `PADDLE_PRICE_ID` and `TOKEN_TTL_SECONDS` are plain
`[vars]`.

### Billing and complimentary access

`plan` is written **only** by the signed Paddle webhook (`comp | trialing | active | past_due |
canceled | expired`): the client can start a checkout but never grant itself access. See
`docs/BILLING.md` in the app repo.

To give someone free, permanent access (family, friends, reviewers):

```bash
ADMIN_TOKEN=… node scripts/comp.mjs grant  friend@example.com "family"
ADMIN_TOKEN=… node scripts/comp.mjs revoke friend@example.com
ADMIN_TOKEN=… node scripts/comp.mjs list
```

A granted email is created as `comp` when it registers; granting an existing account upgrades it
at once. The `/admin/comp` routes return `404` until `ADMIN_TOKEN` is set. Revoking turns an
existing `comp` account read-only. No password is ever involved.

### Looking at the numbers

```bash
node scripts/funnel.mjs        # read-only; uses the wrangler login you already have
```

Prints the billing funnel: accounts, trials started, **trialled → paid**, who is `comp`, who has
never paid, and how much webhook traffic there has been. `trial_used_at` records when a trial
began, which is what makes the conversion a real cohort number — `plan` only ever holds the current
state of an account.

### Upgrading an existing database

New databases get everything from `schema.sql`. An existing one needs each migration once:

```bash
npx wrangler d1 execute lados-sync --remote \
  --command "ALTER TABLE accounts ADD COLUMN scheme TEXT NOT NULL DEFAULT 'legacy'"
npx wrangler d1 execute lados-sync --remote \
  --command "ALTER TABLE vaults ADD COLUMN vault_uuid TEXT"
npx wrangler d1 execute lados-sync --remote \
  --command "CREATE INDEX IF NOT EXISTS idx_vaults_uuid ON vaults(account_id, vault_uuid)"
npx wrangler d1 execute lados-sync --remote \
  --command "CREATE TABLE IF NOT EXISTS rate_limits (bucket TEXT PRIMARY KEY, count INTEGER NOT NULL, reset_at INTEGER NOT NULL)"
npx wrangler d1 execute lados-sync --remote \
  --command "CREATE TABLE IF NOT EXISTS comp_grants (email TEXT PRIMARY KEY, note TEXT, created_at INTEGER NOT NULL)"
```

(Every statement is `IF NOT EXISTS`, so re-running `schema.sql` is also safe.)

## One-time setup (free tier — no card required)

Requires Node.js 20+.

```bash
cd sync-server
npm install
npx wrangler login          # opens the browser to authorize
```

Then create the two resources and wire them up:

```bash
# 1) D1 database (metadata)
npx wrangler d1 create lados-sync
#    → copy the printed "database_id" into wrangler.toml

# 2) R2 bucket (encrypted blobs)
npx wrangler r2 bucket create lados-sync-blobs

# 3) Apply the schema to the remote database
npx wrangler d1 execute lados-sync --file=./schema.sql --remote
```

## Run and deploy

```bash
npm run dev        # local dev server (http://127.0.0.1:8787)
npm run deploy     # publish → prints https://lados-sync.<your-subdomain>.workers.dev
```

No domain is required — the free `*.workers.dev` URL is enough.

Smoke test:

```bash
curl https://lados-sync.<your-subdomain>.workers.dev/health
# {"ok":true}
```

## Free tier

- **Workers**: 100,000 requests/day
- **R2**: 10 GB storage, **no egress fees**
- **D1**: 5 GB, 5M reads/day

Comfortably enough for personal use; subscriptions cover growth.

## Self-hosting

This is a standard Worker — deploy it under your own Cloudflare account, or run
`wrangler dev` behind a tunnel. Nothing here depends on a central service.

## License

AGPL-3.0-only (see `LICENSE`). It may be self-hosted, but not operated as a closed
competing service.
