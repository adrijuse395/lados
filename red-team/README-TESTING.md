# Lados — **test build** (staging)

> **This is not a release.** It is a test build made for a pre-release security assessment of the
> owners' own product (see [`docs/RED-TEAM.md`](../docs/RED-TEAM.md)). **Do not put real passwords
> in it.**

## What this build is

- The Lados desktop app, built with sync enabled and pointed at a **staging** server (not the one
  the product will ship with).
- It is marked **`staging`** in *Settings*, and its **in-app updater is disabled on purpose** — a
  test build must never quietly replace itself with a released one.
- Your vault stays encrypted on this machine. Sync, if you turn it on, uploads only opaque
  ciphertext to the staging server.

## Install and run

| Platform | Asset |
| --- | --- |
| **Linux** | `Lados_0.2.1_amd64.deb` (Debian/Ubuntu) or `Lados_0.2.1_amd64.AppImage` |
| **Windows** | `Lados_0.2.1_x64-setup.exe` (unsigned — SmartScreen will warn; expected) |

The app is a **GUI**. On a headless Linux machine it needs a virtual display:

```bash
xvfb-run -a ./Lados_0.2.1_amd64.AppImage --appimage-extract-and-run
# or, after installing the .deb:  xvfb-run -a lados
```

Then:

1. Launch Lados and create a vault with a **throwaway** master password.
2. To exercise sync: **Sync settings** → sign in with the account you were given.

## The browser extension

- `lados-chrome.zip` — load it unpacked: `chrome://extensions` → **Developer mode** → **Load
  unpacked** → pick the extracted folder.
- It talks only to `127.0.0.1:18765`, so it needs this app running with the vault **unlocked**.
- On a headless machine the browser also needs a virtual display.

## Credentials

Provided to you **separately**. Do not use real, personal accounts on this build.

## Reporting

Use the report format in the task you were given (finding · severity · component · steps · observed ·
expected · evidence · reproducible · impact).
