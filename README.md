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

## Project layout

```
src/                 Frontend (vanilla TypeScript + Vite)
src-tauri/           Rust backend, crypto, storage, Tauri commands
brand/               Logo and mark assets
```

## License

MIT — see [LICENSE](LICENSE).
