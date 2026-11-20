# Task — Functional explorer (Lados, pre-release)

You are an experienced QA/security engineer doing a **pre-release assessment** of a password
manager. You have the app, a browser, and internet access. Your job is to use Lados the way a
demanding, impatient, slightly unlucky user would — and to find where it breaks, behaves oddly, or
risks the user's data.

This is an **authorized** test of the owners' own software. Work only inside the boundaries below.

---

## 1. What Lados is

A fast, offline-first password manager. Each vault is one encrypted `.lados` file on the user's
disk, opened with a master password (optionally plus a key file). All cryptography runs locally
(Argon2id + XChaCha20-Poly1305). There is **no** account and **no** cloud unless the user turns on
the optional sync.

Components you may touch:

- **Desktop app** — the primary product (Windows/macOS/Linux).
- **Browser extension** — fills logins from the local vault over `127.0.0.1:18765`.
- **Sync** (optional, behind a sign-in) — an encrypted copy of each vault on a server, plus paid
  billing. On this test it runs against a **staging** server.

Read the product README and `docs/SYNC.md`, `docs/RECOVERY.md`, `docs/MOBILE.md` for context. The
source repository is public, so you may read it.

## 2. Your environment

- **The app:** you need a build that points at the **staging** server — see *Obtaining the client* below.
- **Extension:** `browser-extension/build/lados-chrome.zip`. Load it **unpacked**
  (`chrome://extensions` → Developer mode → Load unpacked).
- **Test account:** `<TEST_ACCOUNT_EMAIL>` / `<TEST_ACCOUNT_PASSWORD>` (given to you separately).
  Register additional accounts freely if you need them.
- **Staging server:** `<STAGING_URL>` — this is the only server you may use.

### Obtaining the client

There is **no "sandbox" switch in the app**: the server is compiled into the build, so you need one
that was built *for staging*. Two ways to get it:

1. **Pre-release build (preferred).** Download your platform's asset — the direct base URL works with
   `curl -L -O` (a browser works too):
   - base: `<PRE_RELEASE_BASE>`
   - **Linux x86_64:** `<PRE_RELEASE_BASE>/Lados_0.2.2_amd64.deb` (Debian/Ubuntu), or
     `<PRE_RELEASE_BASE>/Lados_0.2.2_amd64.AppImage`.
     The app is a **GUI**: on a headless machine it needs a virtual display, so run it under
     `xvfb-run`, e.g. `xvfb-run -a ./Lados_0.2.2_amd64.AppImage --appimage-extract-and-run`
     (or `sudo apt install ./Lados_0.2.2_amd64.deb` then `xvfb-run -a lados`). The AppImage needs
     FUSE; `--appimage-extract-and-run` avoids it.
   - **Windows x64:** `<PRE_RELEASE_BASE>/Lados_0.2.2_x64-setup.exe` — unsigned, so SmartScreen warns.
   - All assets are listed at `<PRE_RELEASE_URL>`. If none matches your platform, use option 2.
2. **Build from source** (the repository is public, so this is realistic):

   ```bash
   git clone https://github.com/ladospwm/lados.git && cd lados
   npm install
   LADOS_SYNC_SERVER=<STAGING_URL> LADOS_CHANNEL=staging npx tauri build --features sync
   ```

   On Windows PowerShell, set the two variables first:
   `$env:LADOS_SYNC_SERVER = "<STAGING_URL>"` and `$env:LADOS_CHANNEL = "staging"`.

   **Do not build without both values** — a default build points at the *production* server.

**Verify the target before you do anything else.** Open **Settings**: it must show **`staging`** and
the staging server URL. If it shows anything else, **stop** — that build is pointed at production and
must not be used. (The server is remembered per account at sign-in, so use a fresh machine or profile,
or sign out first.)

## 3. Rules (do not cross)

1. **Only the app, the extension, the staging server and the website.** Never the production server,
   Cloudflare itself, or any mail/Paddle system.
2. **Do not destroy data.** Your test account is disposable; nothing else is.
3. **No denial of service, flooding or spam.**
4. **Stop and report** if you encounter data that looks like another person's.
5. **No finding without a reproduction.** A hunch is not a finding — write "could not verify".

## 4. What to test — be exhaustive, then creative

Cover at least all of this, and push each past its intended use:

**Vault lifecycle**
- Create, open, lock, unlock, close, rename; key file (with, without, wrong); change master key.
- Wrong password, no key file, corrupt/truncated/huge file, a file that is not a vault, read-only
  folder, missing folder, path with unusual characters, Unicode names.
- Two windows open on the same vault at once; edit in both; delete the file while open; replace the
  file behind the app's back.

**Entries and folders**
- Create/edit/duplicate/move/delete; nested folders; move a folder into its own child.
- Custom fields (protected and not), icons, expiration dates, very long values, empty values,
  emoji/RTL/combining characters, leading/trailing spaces.
- Entry history and restore. Delete an entry, then restore. Delete a folder with entries in it.
- Search: empty, one character, regex-looking input, very long input, characters that are HTML/SQL.

**Secrets handling**
- Reveal/copy; clipboard auto-clear timing; lock while a secret is on the clipboard; copy from a
  history revision.

**Tools**
- Password generator: every option and combination, extreme lengths, excluded characters that
  exclude everything.
- CSV import/export and KeePass `.kdbx` import/export, with malformed files, duplicates, huge files,
  unexpected encodings; merge two vaults (including merging a vault into itself).

**Recovery keys**
- Create a key; **check** a key; recover with it; recover with a wrong file; recover with a file for
  a different vault; recover while locked out; recover a synced vault while signed in and while
  signed out. Verify the old file stops working after a replacement.

**Sync (staging)**
- Sign up, sign in, wrong password, sign out, sign out everywhere, delete account.
- Enable sync, sync, edit on "two devices" (two vault copies), force a conflict, download a
  cloud-only vault, delete a cloud vault, disable sync.
- Change the account master password and confirm every vault follows.

**Settings, shell, extension**
- Every setting; auto-lock (including with the app in the background); tray; theme; close-to-tray.
- Extension: autofill on several sites, multiple matches, locked vault, app closed, page with
  unusual forms (iframes, shadow DOM, hidden fields, multiple password fields), and the popup.

For each area, ask: *what if the user does the unexpected thing, twice, out of order, or at the
worst moment?*

## 5. What to report

Report anything a real user would hit and resent: **bugs, crashes, wrong/irreversible behaviour,
data-loss risks, confusing or misleading states, missing validation, silent failures, and error
messages that mislead.**

Do **not** report: styling preferences, or anything you cannot reproduce.

Do **not** spend time trying to break authentication, steal data or bypass payment — that is a
different task (`PROMPT-attacker.md`).

## 6. Report format

For each finding:

```
Title:
Severity: informational | low | medium | high | critical
Component: app | extension | API | website
Environment: staging build <version>, <OS>, <browser/OS if relevant>
Preconditions:
Steps to reproduce:      (numbered, exact)
Observed:
Expected:
Evidence:               (screenshot, file, log)
Reproducible:           always | intermittent | once
Impact:                 what a user loses or what goes wrong
Suggested fix (optional):
```

Finish with a **summary**: features covered, features you could not test (and why), and the **top
three** things you would fix before shipping.

## 7. Deliverable

One Markdown report, findings ordered **most severe first**, plus the summary. Be concise and
concrete; skip praise.
