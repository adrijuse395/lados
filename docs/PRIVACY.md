# Lados — Privacy Policy

_Last updated: 17 November 2026_

> This is the source of truth. The same policy is published for the stores at
> `https://<your-domain>/privacy` (the `lados-web` site, `privacy.html`). Keep both in step.

Lados is a local-first password manager. Its design goal is that **your passwords never
leave your computer** unless you deliberately turn on the optional, end-to-end encrypted
Lados Sync.

## In one line

**No accounts required, no trackers, no telemetry, no advertising — and we cannot read
your vaults.**

## The Lados desktop app

- Your vault is an ordinary file on your own computer, encrypted with your master
  password. We never receive it.
- The app works fully offline and makes no network request unless you enable **Lados
  Sync** or ask it to check for updates.
- **Checking for updates** contacts the public release page (GitHub) to see whether a
  newer version exists. It sends nothing about you or your vault.

## The Lados browser extension

- The extension connects **only** to `http://127.0.0.1:18765` — the Lados app running on
  the same computer. It contacts no other server, ours or anyone else's.
- To offer autofill it reads, **locally**, the URL of the page you are on and its
  username/password fields, and asks your local app whether it holds a matching login.
  Only candidate metadata (title, username, URL) is returned; no password is sent to the
  page.
- A password is requested from your local app **only when you click to fill**, and the
  extension never stores it.
- The extension keeps no data, sets no cookies, and contains no analytics. It requests no
  permissions other than the loopback connection, and it ships no remote code.

## Optional: Lados Sync

If you choose to enable Lados Sync, an account (your **email**) and your vaults'
**encrypted** data are stored on our sync server so you can use the same vault on several
devices. Because the encryption key never leaves your device, the server stores only
opaque ciphertext it cannot read. Vault contents, vault names and entry data are never
available to us. The protocol is documented in [`SYNC.md`](./SYNC.md).

## Payments (Lados Sync subscriptions)

Lados Sync is sold by **Paddle**, which acts as the *Merchant of Record*: Paddle is the
seller of record for your subscription and handles the payment, the invoice and any VAT or
sales tax. **We never see or store your card details.**

- **What Paddle processes**, under its own privacy policy (linked from the checkout, and at
  <https://www.paddle.com/legal/privacy>): your email, your payment details, and the billing
  information tax law requires.
- **What we receive and store** is limited to identifiers and state: your account id, Paddle's
  customer and subscription ids, the plan, and the period end. No card data, no address, no tax
  identifier.
- If you never subscribe, none of this exists.

You can stop at any time: cancelling keeps your access until the period you already paid for
ends, and deleting your account cancels the subscription first.

## What we collect

**Nothing.** There is no analytics, no crash reporting, and no data broker sharing. We do
not sell or share personal data. For Lados Sync we store only the email you signed up with
and the encrypted blobs described above; if you subscribe, we additionally store the
payment-processor identifiers listed under *Payments* (never card data).

## Changes

If this policy changes, the new version will be published in this repository with an
updated date.

## Contact

Questions about privacy: **ladospwm@proton.me**
