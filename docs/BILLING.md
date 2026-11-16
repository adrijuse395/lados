# Lados Sync — Billing design

Status: **draft for review**. Supersedes the “Pricing and billing (proposed)” summary in
[`SYNC.md`](./SYNC.md) §11. Entitlement mechanics stay in `SYNC.md` §18.

## 0. Principles

1. **The app is free and offline, forever.** Paying only buys cloud *convenience*, never
   permission to use your own data.
2. **The server is the only authority** on whether an account may sync. The client never
   decides, and may only *display* what the server says.
3. **We never see card data.** A Merchant of Record (MoR) is the seller, so we never handle
   PANs, and VAT/tax paperwork is theirs.
4. **Never hold data hostage.** If a subscription lapses, the user can still **download**
   their encrypted vaults; writing stops. Nothing is deleted.
5. **Identity-agnostic code.** The server stores a plan flag and an opaque customer id. Moving
   from a personal account to a company later changes *accounts*, not code.

## 1. Who is the seller — and why a MoR

Selling digital goods to consumers worldwide means global VAT/GST/OSS, invoicing rules, and
withdrawal rights. A **Merchant of Record** absorbs all of that: it is the legal seller on
the invoice, collects and remits tax, handles refunds and chargebacks, and pays us monthly.

| Provider | Model | Notes |
| --- | --- | --- |
| **Paddle** | MoR | Best-known MoR for SaaS; strong subscription tooling, sandbox, VAT/id handling. Fees ≈ 5% + €0.50 |
| **Polar** | MoR | Developer/open-source oriented (built on Stripe). Fees ≈ 4% + €0.40 |
| **Lemon Squeezy** | MoR | Simplest onboarding; now part of Stripe. Fees ≈ 5% + €0.50 |
| **Stripe (direct)** | *Not* a MoR | Cheapest per-transaction, but **we** would owe VAT worldwide. Too much for a solo project |

**Recommendation: Paddle** (Polar as the leaner alternative). All three are swappable behind
the small interface below — we keep the provider behind `POST /billing/checkout` + one
webhook, so the choice is reversible.

## 2. Identity: keeping Lados separate from the person

Today the repo, the Vercel site, the add-on and the Cloudflare account all sit under a
personal Gmail. Nothing is *wrong*, but we can decouple the **Lados** identity from the
person now, mostly with free, reversible moves:

| Piece | Do this | Why |
| --- | --- | --- |
| Email | Use a domain mailbox, e.g. `hello@` / `billing@` (keep `ladospwm@proton.me` as a fallback) | Every service below signs up with it |
| Domain | Register a dedicated domain for the project | The MoR, the site and the ToS link need a neutral URL |
| GitHub | Create an **organization** and **transfer** the repo into it | Keeps stars/issues/history; org owns the code |
| Vercel | Create a **team** and transfer the site + domain | Site isn’t tied to a personal plan/account |
| Firefox (AMO) | Create the developer account with the org email, then **transfer ownership** of the add-on | Keeps the add-on id (`lados@adrijuse395.github.io` is cosmetic) and its installs |
| Chrome Web Store | Publish under the org account (items can be transferred between accounts) | Same |
| Cloudflare | Ideally a dedicated account/email for the Workers/D1/R2 | Accounts are hard to move later |
| MoR | Register with the **legal seller** identity (see below) | KYC + payouts require a real person or company anyway |

Be honest about the limit: **someone is always the legal seller.** A MoR does not hide the
person. What we *can* do is keep the public brand, the code and the operational accounts
neutral, so a future company (see §9) is a rename, not a rebuild.

**This project is a data/legal matter too** — confirm the entity choice with a Spanish
*gestor*/accountant before taking real money. The design below does not depend on which
entity you pick.

## 3. Product & price

- **One plan: “Lados Sync”**, annual, **€12/year** (≈ €1/month). Annual-only keeps fixed
  per-transaction fees from eating the price.
- **Unlimited** devices and vaults. No artificial caps.
- **Optional 14-day free trial** (see §13) so people can try syncing before paying.
- **Comp plan** for the maintainer/testers — a plan value, not a roles system (`SYNC.md` §11).
- Optional later: a **localized price** (Paddle supports regional pricing) and a
  “founder/lifetime” tier.

## 4. Entitlement states

One column on `accounts`, driven **only** by the webhook:

| `plan` | Meaning | Sync allowed |
| --- | --- | --- |
| `comp` | Maintainer/tester | yes |
| `trialing` | Inside a free trial | yes |
| `active` | Paid and current | yes |
| `past_due` | Renewal failed, inside grace window | yes (until grace ends) |
| `canceled` | Canceled, paid period not over | yes (until `plan_valid_until`) |
| `expired` | Lapsed/refunded/chargeback | **read yes, write no** (see §4.1) |

`plan_valid_until` (epoch ms) is the end of the current paid period, used for display and for
turning `canceled`/`past_due` into `expired`.

### 4.1 Expiry behaviour — recommendation

Today `plan !== "active"` returns `402` on **every** vault call. Recommendation: make expiry
**read-only** instead:

- `GET /vaults`, `GET /vaults/:id` → allowed, so the user can always download their data.
- `PUT /vaults/:id`, `POST /vaults` → `402 Payment Required`.

This is the strongest possible trust signal (“we never keep your data from you”) and it is
cheap to implement. *(Decision needed — see §13.)*

## 5. End-to-end flow

```
User (signed in) clicks “Subscribe”
   │
   ├─▶ POST /billing/checkout                (authenticated, server builds the session)
   │      ◀─ { url }                          provider checkout, carrying custom_data.accountId
   │
   ├─▶ app opens `url` in the default browser  (no card data ever touches Lados)
   │
   │        user pays ──▶ MoR ──(webhook, signed)──▶ POST /billing/webhook
   │                                                     │ verify signature + dedupe event
   │                                                     │ map to account via accountId
   │                                                     ▼
   │                                            accounts.plan = 'active',
   │                                            plan_valid_until = …,
   │                                            billing_customer_id / _subscription_id
   ▼
next sync / GET /me  ◀── { plan: 'active' }  → sync proceeds
```

- The account is bound by **`custom_data.accountId`**, not by email (the receipt email may
  differ). If a purchase ever arrives without it, fall back to matching the MoR customer
  email against `accounts.email`.
- The app **polls** `/me` (or just retries sync) after checkout; there is no push channel.
- Entitlement is **only** ever written by the webhook — never by a client call.

## 6. Webhook contract

`POST /billing/webhook` — no auth header; authenticity comes from the provider signature.

- **Verify** the signature (`Paddle-Signature: ts=…;h1=…`, HMAC-SHA256 over `ts:body`) with a
  secret in `env`. Reject an old `ts` (replay window ≈ 5 min).
- **Dedupe** by event id in a `billing_events` table (insert-or-ignore) so retries are safe.
- Handle at least: `transaction.completed`, `subscription.created|activated|updated|past_due|
  canceled|paused|expired`, `adjustment.created` (refund/chargeback).
- Always answer `200` quickly once recorded (the MoR retries on failure).
- Map provider status → our `plan` (§4) in **one** small function, so the provider swap is a
  single file.

## 7. Data model (minimal, on our side)

```
accounts  (add)
  plan                   TEXT    -- see §4 (already exists; extend the set)
  plan_valid_until       INTEGER
  billing_customer_id    TEXT
  billing_subscription_id TEXT
  billing_provider       TEXT    -- 'paddle' | 'polar' | …
  trial_used_at          INTEGER

billing_events
  id          TEXT PRIMARY KEY   -- provider event id (idempotency)
  received_at INTEGER
```

No names, addresses, or payment methods — those live with the MoR.

## 8. Refunds, chargebacks, dunning

- The MoR issues refunds/cancels and sends the webhook; we flip `plan` accordingly.
- **Grace window** for `past_due` (e.g., 7 days) so a failed renewal doesn’t instantly stop
  sync; the MoR retries the charge during that window.
- A chargeback marks the subscription expired immediately.
- Cancellation never deletes data; the account simply becomes read-only after the paid period.

## 9. Legal pages & entity

Before taking money, the site (`lados-web`) needs—and the MoR will ask for—:

- **Terms of Service**
- **Privacy Policy** (exists: `docs/PRIVACY.md` → `/privacy`) — extend it with billing data
  (see §10)
- **Refund / cancellation policy**
- **Seller identification** (name/company, tax id, address, contact) as Spanish e-commerce
  rules require

Entity: for real revenue this is an **autónomo** or an **SL**. The likely shape is: the
business sells to the **MoR** (B2B, VAT reverse charge), and the MoR sells to end users. All
of this is a *gestor* question — get one before the first payout.

## 10. Privacy of billing data

- We store only: MoR customer/subscription ids and the plan state. No card data, no address.
- The MoR is an independent controller for the payment; link its privacy policy from our
  Privacy page and note the categories (email, payment details) it processes.
- Add a short “Payments” section to `docs/PRIVACY.md` when we wire this up.

## 11. Testing

- MoR **sandbox** + test cards; a `BILLING_SANDBOX=1` env var and a separate webhook secret.
- Automated: replay a recorded event into the webhook behind a flag and assert the resulting
  `plan`/`plan_valid_until` in D1.
- Manual: run a real sandbox subscription through checkout → webhook → `/me` → sync, and a
  renewal, a cancellation, and a refund.

## 12. Rollout plan

1. **Server**: billing columns + `billing_events`, webhook (provider adapter), `/billing/checkout`,
   expose `plan`/`plan_valid_until` in `/me`.
2. **App**: enable the “Manage subscription”/“Subscribe” button → open the checkout URL; render
   the states of §4; handle `402` gracefully (read-only on expiry, per §4.1).
3. **Ops/identity**: the org accounts in §2, the legal pages in §9, a MoR account with sandbox
   then production keys.
4. **Later**: trials, dunning copy, localized pricing, founder tier.

## 13. Open decisions

1. **Provider**: Paddle (recommended) / Polar / Lemon Squeezy?
2. **Price**: €12/year — good? Annual only, or also monthly?
3. **Trial**: 14 days, no card, or none?
4. **Expiry**: read-only after expiry (recommended) or block all cloud calls?
5. **Identity now or later**: start the org/domain move now, or after the first paying users?
6. **Entity**: autónomo vs SL — needs a gestor; do you want a checklist for it?
