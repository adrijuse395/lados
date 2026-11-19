// Seeds a Lados Sync account the desktop app can actually sign in to.
//
// The app never sends the raw master password over the wire. It sends
//
//   hex(Argon2id(composite_key(password), auth_salt(email)))
//
// as the OPAQUE password (`src-tauri/src/crypto.rs`). A seeder that registers the **raw** password
// produces an account the app can never open — the `explorer@example.com` mismatch in the
// red-team report (I1) — so this script reproduces the app's derivation first.
//
//   BASE_URL=https://lados-sync-staging.<subdomain>.workers.dev \
//     node scripts/seed-account.mjs <email> <password>
//
// Registering an email that already exists returns 409. To re-seed one, delete the old row first:
//
//   npx wrangler d1 execute lados-sync-staging --remote \
//     --command "DELETE FROM accounts WHERE email='<email>'"
//
// Self-check (no network), against the value pinned in `src-tauri/src/crypto.rs`:
//
//   node scripts/seed-account.mjs --check

import { sha256 } from "@noble/hashes/sha256";
import { argon2id } from "@noble/hashes/argon2";
import { ready, client } from "@serenity-kit/opaque";

// Must match `SYNC_AUTH_KDF` in crypto.rs: Argon2id, m = 64 MiB, t = 3, p = 4, 32-byte output.
const AUTH_KDF = { t: 3, m: 64 * 1024, p: 4, dkLen: 32 };

const utf8 = (text) => new TextEncoder().encode(text);

function concat(a, b) {
  const out = new Uint8Array(a.length + b.length);
  out.set(a, 0);
  out.set(b, a.length);
  return out;
}

/** `composite_key(password, no key file)` from crypto.rs. */
function compositeKey(password) {
  return sha256(concat(utf8("lados-composite-key-v1"), sha256(utf8(password))));
}

/** The secret the app sends to OPAQUE for this account, as lowercase hex. */
export function authSecret(password, email) {
  const salt = sha256(concat(utf8("lados-sync-auth-salt-v2:"), utf8(email)));
  const secret = argon2id(compositeKey(password), salt, AUTH_KDF);
  return [...secret].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

// The Rust test `sync_auth_secret_matches_the_seed_script` asserts the same value. This is a
// throwaway vector, never a real account's.
const CHECK = {
  password: "seed-vector-password-2026",
  email: "seed-vector@example.test",
  secret: "25f18e825f404e5e374944b011df37ac74bb59f76db379e0a57258f7a87d98ee",
};

function selfCheck() {
  const actual = authSecret(CHECK.password, CHECK.email);
  if (actual !== CHECK.secret) {
    console.error(`derivation drifted\n  expected ${CHECK.secret}\n  actual   ${actual}`);
    process.exit(1);
  }
  console.log("derivation matches the Rust test");
}

async function call(base, method, path, { body } = {}) {
  const headers = {};
  let payload;
  if (body !== undefined) {
    headers["content-type"] = "application/json";
    payload = JSON.stringify(body);
  }
  const response = await fetch(`${base}${path}`, { method, headers, body: payload });
  const text = await response.text();
  let json = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    /* not JSON */
  }
  return { status: response.status, json };
}

async function seed(base, email, password) {
  const secret = authSecret(password, email);
  await ready;
  const registration = client.startRegistration({ password: secret });
  const started = await call(base, "POST", "/auth/register/start", {
    body: { email, registrationRequest: registration.registrationRequest },
  });
  if (started.status !== 200) {
    const detail = started.json?.error ? `: ${started.json.error}` : "";
    console.error(`register/start failed (${started.status})${detail}`);
    process.exit(1);
  }
  const finished = client.finishRegistration({
    password: secret,
    registrationResponse: started.json.registrationResponse,
    clientRegistrationState: registration.clientRegistrationState,
  });
  const done = await call(base, "POST", "/auth/register/finish", {
    body: { email, registrationRecord: finished.registrationRecord },
  });
  if (done.status !== 200) {
    const detail = done.json?.error ? `: ${done.json.error}` : "";
    console.error(`register/finish failed (${done.status})${detail}`);
    process.exit(1);
  }
  console.log(`seeded ${email} — sign in from the app with the master password you gave`);
}

const invokedDirectly = (process.argv[1] || "").replace(/\\/g, "/").endsWith("scripts/seed-account.mjs");
if (invokedDirectly) {
  const args = process.argv.slice(2);
  if (args[0] === "--check") {
    selfCheck();
  } else {
    const [email, password] = args;
    if (!email || !password) {
      console.error("usage: BASE_URL=… node scripts/seed-account.mjs <email> <password>");
      console.error("       node scripts/seed-account.mjs --check");
      process.exit(2);
    }
    const base = (process.env.BASE_URL || "https://lados-sync.ladospwm.workers.dev").replace(/\/+$/, "");
    await seed(base, email, password);
  }
}
