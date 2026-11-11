# Lados Sync server

The **zero-knowledge** sync backend for the [Lados](https://github.com/adrijuse395/lados)
password manager, running as a **Cloudflare Worker**.

It stores an **opaque, encrypted** copy of each vault (in R2) plus minimal metadata
(accounts, versions) in D1. It **never** sees plaintext and never resolves conflicts —
all merging happens on the user's device.

> The matching client lives in the main Lados repo, behind the `sync` Cargo feature.
> Design: `docs/SYNC.md` in that repo.

## Endpoints

| Method | Path | Purpose |
| --- | --- | --- |
| POST | `/auth/register` | `{ email, authSecret }` → `{ token, plan }` |
| POST | `/auth/login` | `{ email, authSecret }` → `{ token, plan }` |
| GET | `/me` | account + plan |
| GET | `/vaults` | list vaults |
| POST | `/vaults` | `{ name }` → create a vault |
| GET | `/vaults/:id` | `{ version, blob }` (base64) |
| PUT | `/vaults/:id` | raw encrypted bytes + `If-Match: <version>` → `{ version }` (409 on conflict) |
| DELETE | `/vaults/:id` | remove a vault |

Auth is a bearer token (opaque, stored hashed, 30-day TTL).

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
