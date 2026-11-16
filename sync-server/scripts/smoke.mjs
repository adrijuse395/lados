// End-to-end smoke test for the Lados Sync worker.
//
//   BASE_URL=https://your-worker.workers.dev node scripts/smoke.mjs
//
// It creates throwaway accounts through OPAQUE (exactly like the app), exercises the whole API,
// and deletes everything afterwards. Exits non-zero if any check fails.

import { ready, client } from "@serenity-kit/opaque";

const BASE = (process.env.BASE_URL || "https://lados-sync.ladospwm.workers.dev").replace(/\/+$/, "");

let pass = 0;
const failures = [];

function check(name, condition, info = "") {
  if (condition) {
    pass += 1;
    console.log(`  ok   ${name}`);
  } else {
    failures.push(name);
    console.log(` FAIL  ${name}${info ? `  — ${info}` : ""}`);
  }
}

async function call(method, path, { token, body, ifMatch, raw } = {}) {
  const headers = {};
  if (token) headers.authorization = `Bearer ${token}`;
  if (ifMatch !== undefined) headers["if-match"] = String(ifMatch);
  let payload;
  if (raw) {
    headers["content-type"] = "application/octet-stream";
    payload = raw;
  } else if (body !== undefined) {
    headers["content-type"] = "application/json";
    payload = JSON.stringify(body);
  }
  const response = await fetch(`${BASE}${path}`, { method, headers, body: payload });
  const text = await response.text();
  let json = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    /* not JSON */
  }
  return { status: response.status, json, text };
}

const rid = () => Math.random().toString(36).slice(2, 10);

/// Registers a brand-new account through OPAQUE. Returns the response status and, on success,
/// a session token.
async function register(email, password) {
  const registration = client.startRegistration({ password });
  const started = await call("POST", "/auth/register/start", {
    body: { email, registrationRequest: registration.registrationRequest },
  });
  if (started.status !== 200) return { status: started.status, token: undefined };
  const finished = client.finishRegistration({
    password,
    registrationResponse: started.json.registrationResponse,
    clientRegistrationState: registration.clientRegistrationState,
  });
  const done = await call("POST", "/auth/register/finish", {
    body: { email, registrationRecord: finished.registrationRecord },
  });
  return { status: done.status, token: done.json?.token };
}

/// Logs in through OPAQUE. A wrong password is refused by the client, so it never reaches the
/// server's finish step.
async function login(email, password) {
  const start = client.startLogin({ password });
  const first = await call("POST", "/auth/login/start", {
    body: { email, startLoginRequest: start.startLoginRequest },
  });
  if (first.status !== 200) return { status: first.status, token: undefined };
  const finished = client.finishLogin({
    password,
    loginResponse: first.json.loginResponse,
    clientLoginState: start.clientLoginState,
  });
  if (!finished) return { status: 401, token: undefined };
  const done = await call("POST", "/auth/login/finish", {
    body: { loginId: first.json.loginId, finishLoginRequest: finished.finishLoginRequest },
  });
  return { status: done.status, token: done.json?.token };
}

const emailA = `smoke-${rid()}@example.com`;
const emailB = `smoke-${rid()}@example.com`;
const passwordA = `smoke-pass-a-${rid()}`;
const passwordB = `smoke-pass-b-${rid()}`;
const uuidA = crypto.randomUUID();
const uuidB = crypto.randomUUID();

await ready;
console.log(`Testing ${BASE}\n`);

// ----- Health and authentication -----
let r = await call("GET", "/health");
check("health responds ok", r.status === 200 && r.json?.ok === true);

r = await call("GET", "/vaults");
check("unauthenticated request is rejected (401)", r.status === 401);

let auth = await register(emailA, passwordA);
const tokenA = auth.token;
check("register returns a token", auth.status === 200 && typeof tokenA === "string");

auth = await register(emailA, passwordA);
check("registering the same email twice is refused (409)", auth.status === 409);

auth = await login(emailA, passwordA);
check("login with the right password succeeds", auth.status === 200 && typeof auth.token === "string");

auth = await login(emailA, `wrong-${rid()}`);
check("login with a wrong password is refused (401)", auth.status === 401);

r = await call("GET", "/me", { token: tokenA });
check(
  "me returns a usable plan",
  r.status === 200 && ["active", "trialing", "comp"].includes(r.json?.plan),
  `plan ${r.json?.plan}`,
);

// ----- Cloud vaults keyed by the vault's own uuid -----
r = await call("POST", "/vaults", { token: tokenA, body: { vaultUuid: uuidA, nameCipher: "cipher-alpha" } });
const vaultA = r.json?.id;
check("create a cloud vault", r.status === 200 && typeof vaultA === "string");

r = await call("POST", "/vaults", { token: tokenA, body: { vaultUuid: uuidA, nameCipher: "cipher-alpha" } });
check("the same vault uuid maps to the same cloud vault", r.json?.id === vaultA, `got ${r.json?.id}`);

r = await call("POST", "/vaults", { token: tokenA, body: { vaultUuid: uuidB, nameCipher: "cipher-beta" } });
const vaultB = r.json?.id;
check("a different uuid creates its own cloud vault", r.status === 200 && vaultB && vaultB !== vaultA);

r = await call("GET", "/vaults", { token: tokenA });
check("the account has two cloud vaults", r.status === 200 && r.json?.vaults?.length === 2, `got ${r.json?.vaults?.length}`);
check(
  "the encrypted label is stored and returned",
  Array.isArray(r.json?.vaults) && r.json.vaults.some((vault) => vault.name_cipher === "cipher-alpha"),
);

// ----- Upload / download / concurrency -----
r = await call("PUT", `/vaults/${vaultA}`, { token: tokenA, ifMatch: 0, raw: new Uint8Array(700) });
check("first upload stores version 1", r.status === 200 && r.json?.version === 1, `status ${r.status}`);

r = await call("GET", `/vaults/${vaultA}`, { token: tokenA });
check("download returns version 1 with a blob", r.status === 200 && r.json?.version === 1 && r.json?.blob?.length > 0);

r = await call("PUT", `/vaults/${vaultA}`, { token: tokenA, ifMatch: 99, raw: new Uint8Array(700) });
check("upload with a stale version is a conflict (409)", r.status === 409);

r = await call("PUT", `/vaults/${vaultA}`, { token: tokenA, ifMatch: 1, raw: new Uint8Array(800) });
check("second upload stores version 2", r.status === 200 && r.json?.version === 2);

r = await call("GET", `/vaults/${vaultA}`, { token: tokenA });
check("current blob survives the cleanup of the old one", r.status === 200 && r.json?.version === 2);

r = await call("PUT", `/vaults/${vaultA}`, { token: tokenA, ifMatch: 2, raw: new Uint8Array(0) });
check("an empty blob is refused (400)", r.status === 400);

r = await call("PUT", `/vaults/${vaultA}`, { token: tokenA, ifMatch: 2, raw: new Uint8Array(9 * 1024 * 1024) });
check("an oversized blob is refused (413)", r.status === 413, `status ${r.status}`);

// ----- Accounts are isolated -----
auth = await register(emailB, passwordB);
const tokenB = auth.token;
check("register a second account", auth.status === 200 && typeof tokenB === "string");

r = await call("GET", `/vaults/${vaultA}`, { token: tokenB });
check("another account cannot read a vault (404)", r.status === 404);

r = await call("PUT", `/vaults/${vaultA}`, { token: tokenB, ifMatch: 2, raw: new Uint8Array(700) });
check("another account cannot write a vault (404)", r.status === 404);

r = await call("GET", "/vaults", { token: tokenB });
check("another account sees no vaults", r.status === 200 && r.json?.vaults?.length === 0);

// ----- Deleting a vault and an account -----
r = await call("DELETE", `/vaults/${vaultB}`, { token: tokenA });
check("delete a cloud vault", r.status === 200);

r = await call("GET", "/vaults", { token: tokenA });
check("the account now has one cloud vault", r.status === 200 && r.json?.vaults?.length === 1);

r = await call("DELETE", "/account", { token: tokenA });
check("delete the account", r.status === 200);

r = await call("GET", "/vaults", { token: tokenA });
check("the token stops working after deletion (401)", r.status === 401);

auth = await login(emailA, passwordA);
check("the deleted account cannot log in (401)", auth.status === 401);

r = await call("DELETE", "/account", { token: tokenB });
check("clean up the second account", r.status === 200);

// ----- Sign out everywhere -----
const soEmail = `smoke-so-${rid()}@example.com`;
const soPassword = `smoke-pass-so-${rid()}`;
auth = await register(soEmail, soPassword);
const soToken1 = auth.token;
auth = await login(soEmail, soPassword);
const soToken2 = auth.token;
check("create two sessions for one account", typeof soToken1 === "string" && typeof soToken2 === "string");
r = await call("DELETE", "/sessions", { token: soToken2 });
check("sign out everywhere succeeds", r.status === 200);
r = await call("GET", "/me", { token: soToken1 });
check("the other session is revoked (401)", r.status === 401);
r = await call("GET", "/me", { token: soToken2 });
check("the current session still works", r.status === 200);
r = await call("DELETE", "/account", { token: soToken2 });
check("clean up the sign-out account", r.status === 200);

// ----- Rate limiting -----
const rlEmail = `smoke-rl-${rid()}@example.com`;
auth = await register(rlEmail, `smoke-pass-rl-${rid()}`);
const tokenRl = auth.token;
check("register a rate-limit test account", auth.status === 200 && typeof tokenRl === "string");
let limited = false;
for (let i = 0; i < 30 && !limited; i++) {
  const attempt = await login(rlEmail, `nope-${rid()}`);
  if (attempt.status === 429) limited = true;
}
check("repeated login attempts are rate limited (429)", limited);
if (tokenRl) {
  r = await call("DELETE", "/account", { token: tokenRl });
  check("clean up the rate-limit account", r.status === 200);
}

console.log(`\n${pass} passed, ${failures.length} failed`);
if (failures.length > 0) {
  console.log(`Failures: ${failures.join(", ")}`);
  process.exit(1);
}
