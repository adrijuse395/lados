# Lados — go-live runbook

> **Not now.** This is a reference for launch day, not the next piece of work. There is no released
> build that contains sync, and the paid product is not on sale — so nothing here is urgent, and
> starting it early only leaves half-configured state to keep track of. Come back to it when the app
> is genuinely ready to ship.

The design lives in [`BILLING.md`](./BILLING.md); this is the **ordered** sequence for turning
Lados Sync on for real money. Work top to bottom: several steps are manual reviews, and the order
matters — in particular, **sync must not ship in a release before the live credentials are in**
(see Phase 4), or real users would meet a *test-mode* checkout they cannot pay in.

Mark a box only when you have the evidence shown next to it.

---

## Phase 0 — decisions to settle first

**Lados is a brand, not a legal person.** `ladospwm` is a name the work trades under; there is no
company behind it. So "the seller" is a **natural person** — whoever owns the product — unless an
entity is created before this phase. That is not a blocker in itself: **Paddle accepts sole
traders**, and its domain review asks for *“the company name or sole proprietor's brand (legal name
preferred for sole proprietors)”*. The **legal name** would simply be that person's own name, and
the **tax id** their NIF.

Whether registering is *required* in order to invoice a Spanish merchant of record is a **gestor
question**, and it belongs here rather than being guessed at.

- [ ] **Who is the seller**: the owner as an individual, or a new entity. If an entity, allow time
      for it to exist before Paddle's verification.
- [ ] **Seller details**, in whichever form: legal name, tax id, postal address, contact email.
      *(The Terms page needs these — not a company name that does not exist.)*
- [ ] **The grandfathered account**: `st***@gmail.com` is `active` with no subscription — free by
      accident, because it predates billing. Decide now, or it stays free forever:
      - free on purpose → give it a `comp` grant (`node scripts/comp.mjs grant <email> "founder"`);
      - not free → expire it (Phase 3, last step).

## Phase 1 — the live Paddle account *(manual review: start today)*

- [ ] **Verification (KYC)** submitted and **approved** for the business identity.
      *Done when:* the dashboard stops saying the account is unverified.
- [ ] **Checkout domain approved** for the domain you sell from — **My account → Settings → Website
      approval** → *Add a new domain* → *Submit for Approval*. Most are approved automatically; a
      manual review is quoted at **5–7 business days**, so submit early. Sandbox approves
      automatically; live does not, and **checkout will not open on an unapproved domain**.
      *Done when:* the domain shows as approved (dashboard, or the **List checkout domains** API).
      *Paddle checks the site itself:* a clear description of the product, **pricing**, the key
      features, and **Terms + refund policy + privacy reachable from the navigation** — see Phase 2,
      which must therefore be done *first*.
- [ ] **Payout / bank details** filled in.
- [ ] **Product + price created in live**: the live twin of “Lados Sync”, €12/year, VAT included.
      *Write down the live `pri_…`* — it is not secret, and Phase 3 needs it.
- [ ] **Notification destination created in live**, pointing at
      `https://lados-sync.ladospwm.workers.dev/billing/webhook`, subscribed to the same events as the
      sandbox destination. *Write down its `pdl_ntfset_…` secret* — Phase 3 needs it (keep it out of
      the chat and out of git).
- [ ] **Live client-side token** created for the website. *Write it down* (it is public).
- [ ] **Terms + refund policy URLs set in Paddle** — see Phase 2.

## Phase 2 — the legal pages go live

Two things the site needs, and both can only exist **once there is a real seller identity**:
publishing legal terms without one is worse than having none.

- [ ] **Seller details decided** (Phase 0), then fill `[LEGAL NAME]`, `[NIF/CIF]`, `[ADDRESS]`,
      `[CONTACT EMAIL]` in `lados-web/terms.html` and remove the `BEFORE THIS PAGE GOES LIVE`
      comment.
- [ ] …and reviewed by a gestor or lawyer **before** real money is taken.
- [ ] **Link** `terms.html` from the footer of `index.html` and `privacy.html` (plus a *Refunds*
      link to `/terms#refunds`), and add a **pricing section** to `index.html`. Paddle's domain
      review looks for exactly that.
      *A first attempt was reverted (`e99ff46` → `faa4c03`) because it went live with placeholders
      while no shipped build contains sync at all; `git revert faa4c03` brings it back.*
- [ ] The site deployed and `https://<domain>/terms` shows the real seller identity.
- [ ] The URLs pasted into Paddle: the Terms, and `/terms#refunds` as the refund policy.

## Phase 3 — swap the credentials

Do these **together**, then verify. Never leave the system half-swapped.

- [ ] Worker secret `PADDLE_API_KEY` → the **live** key.
      Dashboard → *Workers & Pages → lados-sync → Settings → Variables and Secrets* → edit → Save →
      Deploy. (Set it as a **Secret**, never a plaintext variable; a `wrangler deploy` drops
      dashboard-only plaintext variables.)
- [ ] Worker secret `PADDLE_WEBHOOK_SECRET` → the **live** destination secret.
- [ ] `sync-server/wrangler.toml` → `PADDLE_PRICE_ID` = the **live** `pri_…`, then deploy:
      `cd sync-server && npx wrangler deploy`.
- [ ] `lados-web/main.js` → `PADDLE_CLIENT_TOKEN` = the **live** token, and
      `PADDLE_SANDBOX = false`. Commit and push; Vercel redeploys.
- [ ] Sweep knowingly-free accounts (Phase 0 decision):
      `npx wrangler d1 execute lados-sync --remote --command "UPDATE accounts SET plan='expired', plan_valid_until=NULL WHERE plan='active' AND billing_subscription_id IS NULL"`

### Verify the swap *(do not skip: each line is evidence)*

```powershell
# 1. The Worker still answers, and its checkout now targets live Paddle.
#    Expect 200 and a checkout URL. A 503 means a required setting is missing;
#    a 502 means Paddle refused (check paddleStatus in the body).
curl.exe -s -X POST https://lados-sync.ladospwm.workers.dev/billing/checkout -H "authorization: Bearer <token>"

# 2. A bogus webhook signature is rejected as a *signature* problem, not a missing setting.
#    Expect 401 invalid_signature. A 503 means PADDLE_WEBHOOK_SECRET is missing.
curl.exe -s -o NUL -w "%{http_code}`n" -X POST https://lados-sync.ladospwm.workers.dev/billing/webhook -H "content-type: application/json" -H "paddle-signature: ts=1;h1=deadbeef" -d "{}"

# 3. End-to-end, with a real card you are willing to refund: buy once, then
node sync-server/scripts/funnel.mjs     # expect: trials started 1, trialled -> paid 1/1 (100.0%)
```

- [ ] Checkout returns 200 (not 503/502).
- [ ] Webhook returns 401 for a bad signature (not 503).
- [ ] One real purchase completes, the plan flips to `active`, and Paddle's dashboard shows the
      webhook delivered as **200**.
- [ ] Refund that purchase (it proved the path; do not keep the money).
- [ ] `/billing/cancel` and `/billing/resume` still return 200.

## Phase 4 — ship a build that contains sync

Only now. Until this point every release has `sync` **off**, and that is why none of them can show a
paywall.

- [ ] `src-tauri/Cargo.toml` version bumped (e.g. `0.3.0`), and `package.json`,
      `tauri.conf.json` kept in step.
- [ ] `.github/workflows/release.yml`: add `--features sync` to **each** entry in the build matrix's
      `args` (Windows, Linux, `macos-aarch64`, `macos-x86_64`).
      *This is the line that turns sync on for the public.*
- [ ] `dist` is clean before the build runs — `npm run build` empties it, so the check is that no
      `*.js.map` and no orphan bundles are left from an earlier build (they would be embedded in the
      shipped app):
      `Get-ChildItem dist -Recurse -File | Select-Object Name`
      *Expected:* one `index-*.js`, one `index-*.css`, `index.html`, and the images — nothing else.
- [ ] `browser-extension` version bumped and re-zipped (`python3 scripts/build-extension.py`) if the
      extension changed.
- [ ] Tag and push: `git tag -a v0.3.0 -m "Release v0.3.0" && git push origin v0.3.0`.
- [ ] The release workflow finished for all four platforms, and the installer for at least your own
      platform launches, shows Sync, and can sign in.
      *Done when:* you have actually signed in to a **released** build and seen the account badge.

## Phase 5 — after the first real sales

- [ ] `node sync-server/scripts/funnel.mjs` — the trial→paid number is now meaningful. Record it.
- [ ] Confirm a renewal is scheduled in Paddle and that the plan's `planValidUntil` matches.
- [ ] Watch for deliverability: `billing_unavailable` or repeated 502s in the Worker logs.
- [ ] Keep the sandbox destination alive for testing; only the live one matters for revenue.

---

## Rollback

If Phase 3 goes wrong, put the sandbox credentials back (Worker secrets, `PADDLE_PRICE_ID`,
`PADDLE_CLIENT_TOKEN` + `PADDLE_SANDBOX = true`) and redeploy. **Nobody can be charged in sandbox**,
so this is always a safe place to land. Any live subscription already created must be cancelled by
hand in Paddle.

## Things that are deliberately *not* in this runbook

- **Trial abuse**: signup is free (email + password, no card) and grants 14 days. Bounded by rate
  limits and the 100 MB quota; revisit only if it actually shows up in the funnel.
- **Data durability**: D1 has Time Travel (30 days) built in. Confirm what protection R2 has for the
  encrypted blobs before you rely on it, and decide if a copy elsewhere is warranted.
- **The errors-only log**: not implemented. Until it is, the way to capture a fault is the message
  the interface shows (a screenshot is enough).
