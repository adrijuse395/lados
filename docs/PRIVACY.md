# Lados — Privacy Policy

_Last updated: 16 November 2026_

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

## What we collect

**Nothing.** There is no analytics, no crash reporting, and no data broker sharing. We do
not sell or share personal data. For Lados Sync we store only the email you signed up with
and the encrypted blobs described above.

## Changes

If this policy changes, the new version will be published in this repository with an
updated date.

## Contact

Questions about privacy: **ladospwm@proton.me**
