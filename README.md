# Lados

**Lados** is a fast, offline-first password manager for desktop. Your vault stays on your machine — there is no cloud account, no sync server, and no telemetry.

The name comes from [Ladon](https://en.wikipedia.org/wiki/Ladon_(mythology)), the dragon that guarded the golden apples of the Hesperides: quiet custody, not conquest.

## Features

- Local encrypted vaults (`.lados`)
- Master password and optional key file
- Nested folders, entry history, and soft-delete tombstones
- Smart merge when combining two vaults
- CSV and KeePass (`.kdbx`) import / export
- Password generator, clipboard auto-clear, and auto-lock
- Cross-platform desktop shell (Windows, macOS, Linux) via [Tauri](https://tauri.app/) v2

## Cryptography

All cryptography runs in the Rust backend. The TypeScript UI never sees raw key material beyond what you type into the unlock form.

| Layer | Choice |
| --- | --- |
| Key derivation | **Argon2id** (parameters stored in the vault header so costs can rise later) |
| Authenticated encryption | **XChaCha20-Poly1305** |
| Composite key | Master password, optional key file, or both |
| Memory hygiene | Sensitive buffers are zeroized where practical |

Vaults are sealed offline. Opening a `.lados` file never requires a network.

## Requirements

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://rustup.rs/) 1.90+
- Platform webview:
  - **Windows:** WebView2 (usually preinstalled)
  - **macOS:** system WebKit
  - **Linux:** `webkit2gtk` (distro package)

## Develop

```bash
git clone https://github.com/adrijuse395/lados.git
cd lados
npm install
npm run tauri dev
```

## Build

```bash
npm install
npm run tauri build
```

Installers and the release binary are written under `src-tauri/target/release/` (and `bundle/` for MSI/NSIS and other targets).

## Test

```bash
# Rust unit tests
cd src-tauri && cargo test

# TypeScript typecheck
npm run typecheck
```

Continuous integration runs those checks on every push and pull request to `main`.

## Releasing

Version numbers live in `package.json` and `src-tauri/tauri.conf.json` (keep them in sync). To cut a release:

```bash
# after merging to main and bumping the version fields
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin v0.1.0
```

GitHub Actions then builds installers for Windows, macOS (Apple Silicon and Intel), and Linux, signs updater artifacts, uploads `latest.json`, and attaches everything to a **draft** GitHub Release for that tag. Review the draft, then publish it.

### Updater signing secrets

Releases must sign update packages. The public key lives in `src-tauri/tauri.conf.json`. The private key is a repository secret:

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of the private key file |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password if the key has one (omit or leave empty otherwise) |

Installed apps check `https://github.com/adrijuse395/lados/releases/latest/download/latest.json` and can download/install the new build in-app. Vault data never leaves the machine; only that HTTPS check and the installer download use the network.

## Project layout

```
src/                 Frontend (vanilla TypeScript + Vite)
src-tauri/           Rust backend, crypto, storage, Tauri commands
brand/               Logo and mark assets
```

## License

[GPL-3.0-only](LICENSE) — Copyright (C) 2026 Adrià Junquera.

This program is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, version 3.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
