// Lados Sync — minimal, zero-knowledge sync API.
//
// The server stores an OPAQUE, encrypted copy of each vault and never sees
// plaintext. It authenticates accounts and tracks versions for optimistic
// concurrency. All merging happens on the client (see docs/SYNC.md in the app repo).
//
// SPDX-License-Identifier: AGPL-3.0-only

import init, { create_registration_response, finish_login, start_login } from "../opaque-wasm/pkg/lados_opaque.js";
import wasmModule from "../opaque-wasm/pkg/lados_opaque_bg.wasm";

export interface Env {
  DB: D1Database;
  BLOBS: R2Bucket;
  TOKEN_TTL_SECONDS: string;
  /** OPAQUE server setup (OPRF seed + key pair). A secret; its leak enables offline guessing. */
  OPAQUE_SERVER_SETUP: string;
  /** Billing (Paddle). Absent until configured, which keeps the whole flow inert. See docs/BILLING.md. */
  PADDLE_API_KEY?: string;
  PADDLE_WEBHOOK_SECRET?: string;
  PADDLE_PRICE_ID?: string;
}

// The OPAQUE WASM is imported as a module (compiled at deploy time) and instantiated once.
await init({ module_or_path: wasmModule });

const enc = new TextEncoder();

/// Total encrypted storage an account may hold (all its vaults). A password vault is a few
/// hundred KB, so this is very generous while still bounding abuse.
const ACCOUNT_QUOTA_BYTES = 100 * 1024 * 1024;

function bearerToken(request: Request): string | null {
  const match = (request.headers.get("authorization") || "").match(/^Bearer\s+(.+)$/i);
  return match ? match[1] : null;
}

function json(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function toHex(bytes: ArrayBuffer): string {
  let out = "";
  for (const byte of new Uint8Array(bytes)) out += byte.toString(16).padStart(2, "0");
  return out;
}

function randomHex(byteLength: number): string {
  return toHex(crypto.getRandomValues(new Uint8Array(byteLength)).buffer);
}

async function sha256Hex(value: string): Promise<string> {
  return toHex(await crypto.subtle.digest("SHA-256", enc.encode(value)));
}

/// The stored credential is a one-way function of the client's high-entropy auth
/// secret (the expensive Argon2 work already happened on the client). A salt makes
/// identical secrets produce different verifiers.
async function verifierFor(salt: string, authSecret: string): Promise<string> {
  return sha256Hex(`${salt}:${authSecret}`);
}

function timingSafeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

interface AccountRow {
  id: string;
  email: string;
  plan: string;
  plan_valid_until: number | null;
  billing_subscription_id: string | null;
  password_changed_at: number | null;
}

/// Creates a fresh session token for an account and returns it.
async function startSession(env: Env, accountId: string): Promise<string> {
  const token = randomHex(32);
  const tokenHash = await sha256Hex(token);
  const ttl = Number(env.TOKEN_TTL_SECONDS || "2592000");
  const expiresAt = Date.now() + ttl * 1000;
  await env.DB.prepare(
    "INSERT INTO sessions (token_hash, account_id, expires_at) VALUES (?, ?, ?)",
  )
    .bind(tokenHash, accountId, expiresAt)
    .run();
  return token;
}

async function authenticate(env: Env, request: Request): Promise<AccountRow | null> {
  const token = bearerToken(request);
  if (!token) return null;
  const tokenHash = await sha256Hex(token);
  const row = await env.DB.prepare(
    `SELECT a.id AS id, a.email AS email, a.plan AS plan, a.plan_valid_until AS plan_valid_until, a.billing_subscription_id AS billing_subscription_id, a.password_changed_at AS password_changed_at
       FROM sessions s JOIN accounts a ON a.id = s.account_id
      WHERE s.token_hash = ? AND s.expires_at > ?`,
  )
    .bind(tokenHash, Date.now())
    .first<AccountRow>();
  return row ?? null;
}

async function readJson<T>(request: Request): Promise<T | null> {
  try {
    return (await request.json()) as T;
  } catch {
    return null;
  }
}

// ----- Handlers -----

function clientIp(request: Request): string {
  return request.headers.get("cf-connecting-ip") || "unknown";
}

/// Increments a fixed-window counter and returns false once the limit is reached. Best-effort
/// (a lost race can let one extra request through), which is fine for abuse throttling.
async function allow(env: Env, bucket: string, limit: number, windowSeconds: number): Promise<boolean> {
  const now = Date.now();
  const resetAt = now + windowSeconds * 1000;
  const row = await env.DB.prepare("SELECT count, reset_at FROM rate_limits WHERE bucket = ?")
    .bind(bucket)
    .first<{ count: number; reset_at: number }>();
  if (!row || row.reset_at <= now) {
    await env.DB.prepare(
      "INSERT INTO rate_limits (bucket, count, reset_at) VALUES (?, 1, ?) ON CONFLICT(bucket) DO UPDATE SET count = 1, reset_at = ?",
    )
      .bind(bucket, resetAt, resetAt)
      .run();
    // Cheap opportunistic cleanup of expired buckets.
    if (Math.random() < 0.02) {
      await env.DB.prepare("DELETE FROM rate_limits WHERE reset_at <= ?").bind(now).run();
    }
    return true;
  }
  if (row.count >= limit) return false;
  await env.DB.prepare("UPDATE rate_limits SET count = count + 1 WHERE bucket = ?").bind(bucket).run();
  return true;
}

async function register(env: Env, request: Request): Promise<Response> {
  const body = await readJson<{ email?: string; authSecret?: string }>(request);
  const email = body?.email?.trim().toLowerCase();
  const authSecret = body?.authSecret;
  if (!email || !email.includes("@") || !authSecret) {
    return json({ error: "invalid_request" }, 400);
  }

  if (!(await allow(env, `register:ip:${clientIp(request)}`, 30, 900))) {
    return json({ error: "rate_limited" }, 429);
  }

  const existing = await env.DB.prepare("SELECT id FROM accounts WHERE email = ?")
    .bind(email)
    .first();
  if (existing) return json({ error: "email_taken" }, 409);

  const salt = randomHex(16);
  const verifier = await verifierFor(salt, authSecret);
  const id = crypto.randomUUID();
  const start = initialPlan(env);
  await env.DB.prepare(
    "INSERT INTO accounts (id, email, salt, verifier, scheme, plan, plan_valid_until, created_at) VALUES (?, ?, ?, ?, 'argon2v2', ?, ?, ?)",
  )
    .bind(id, email, salt, verifier, start.plan, start.validUntil, Date.now())
    .run();

  const token = await startSession(env, id);
  return json({ token, plan: start.plan });
}

async function login(env: Env, request: Request): Promise<Response> {
  const body = await readJson<{
    email?: string;
    authSecret?: string;
    previousAuthSecret?: string;
    legacyAuthSecret?: string;
  }>(request);
  const email = body?.email?.trim().toLowerCase();
  const authSecret = body?.authSecret;
  if (!email || !authSecret) return json({ error: "invalid_request" }, 400);

  if (!(await allow(env, `login:ip:${clientIp(request)}`, 100, 900))) return json({ error: "rate_limited" }, 429);
  if (!(await allow(env, `login:email:${email}`, 20, 900))) return json({ error: "rate_limited" }, 429);

  const row = await env.DB.prepare(
    "SELECT id, salt, verifier, scheme, plan FROM accounts WHERE email = ?",
  )
    .bind(email)
    .first<{ id: string; salt: string; verifier: string; scheme: string; plan: string }>();
  if (!row) return json({ error: "invalid_credentials" }, 401);

  // Each scheme has its own secret; the client sends the matching one so older accounts can
  // authenticate once and be upgraded to the per-account salt.
  let presented: string | undefined;
  if (row.scheme === "argon2v2") presented = authSecret;
  else if (row.scheme === "argon2") presented = body?.previousAuthSecret ?? authSecret;
  else presented = body?.legacyAuthSecret;
  if (!presented) return json({ error: "invalid_credentials" }, 401);

  const candidate = await verifierFor(row.salt, presented);
  if (!timingSafeEqual(candidate, row.verifier)) return json({ error: "invalid_credentials" }, 401);

  if (row.scheme !== "argon2v2") {
    const salt = randomHex(16);
    const verifier = await verifierFor(salt, authSecret);
    await env.DB.prepare("UPDATE accounts SET salt = ?, verifier = ?, scheme = 'argon2v2' WHERE id = ?")
      .bind(salt, verifier, row.id)
      .run();
  }

  const token = await startSession(env, row.id);
  return json({ token, plan: row.plan });
}

// ----- OPAQUE auth (the server never sees a password-equivalent) -----

/// How long an in-flight login may take before its state is discarded.
const LOGIN_TTL_MS = 2 * 60 * 1000;

/// Registration, step 1: the server's OPRF response to the client's request.
async function opaqueRegisterStart(env: Env, request: Request): Promise<Response> {
  const body = await readJson<{ email?: string; registrationRequest?: string }>(request);
  const email = body?.email?.trim().toLowerCase();
  if (!email || !email.includes("@") || !body?.registrationRequest) {
    return json({ error: "invalid_request" }, 400);
  }
  if (!(await allow(env, `reg:ip:${clientIp(request)}`, 30, 900))) return json({ error: "rate_limited" }, 429);

  const existing = await env.DB.prepare("SELECT id FROM accounts WHERE email = ?").bind(email).first();
  if (existing) return json({ error: "email_taken" }, 409);

  const registrationResponse = create_registration_response(env.OPAQUE_SERVER_SETUP, email, body.registrationRequest);
  return json({ registrationResponse });
}

/// Registration, step 2: store the registration record and start a session.
async function opaqueRegisterFinish(env: Env, request: Request): Promise<Response> {
  const body = await readJson<{ email?: string; registrationRecord?: string }>(request);
  const email = body?.email?.trim().toLowerCase();
  if (!email || !email.includes("@") || !body?.registrationRecord) {
    return json({ error: "invalid_request" }, 400);
  }
  const existing = await env.DB.prepare("SELECT id FROM accounts WHERE email = ?").bind(email).first();
  if (existing) return json({ error: "email_taken" }, 409);

  const id = crypto.randomUUID();
  const start = initialPlan(env);
  await env.DB.prepare(
    "INSERT INTO accounts (id, email, salt, verifier, scheme, opaque_record, plan, plan_valid_until, created_at) VALUES (?, ?, '', '', 'opaque', ?, ?, ?, ?)",
  )
    .bind(id, email, body.registrationRecord, start.plan, start.validUntil, Date.now())
    .run();
  const token = await startSession(env, id);
  return json({ token, plan: start.plan });
}

/// Login, step 1: the server response plus a short-lived id holding its handshake state.
async function opaqueLoginStart(env: Env, request: Request): Promise<Response> {
  const body = await readJson<{ email?: string; startLoginRequest?: string }>(request);
  const email = body?.email?.trim().toLowerCase();
  if (!email || !body?.startLoginRequest) return json({ error: "invalid_request" }, 400);
  if (!(await allow(env, `login:ip:${clientIp(request)}`, 100, 900))) return json({ error: "rate_limited" }, 429);
  if (!(await allow(env, `login:email:${email}`, 20, 900))) return json({ error: "rate_limited" }, 429);

  const account = await env.DB.prepare("SELECT id, opaque_record FROM accounts WHERE email = ?")
    .bind(email)
    .first<{ id: string; opaque_record: string | null }>();

  // No record -> a dummy response, so we do not reveal whether the email is registered.
  const seed = crypto.getRandomValues(new Uint8Array(32));
  const result = JSON.parse(
    start_login(env.OPAQUE_SERVER_SETUP, account?.opaque_record ?? null, body.startLoginRequest, email, seed),
  ) as { serverLoginState: string; loginResponse: string };

  const loginId = randomHex(16);
  await env.DB.prepare("INSERT INTO logins (id, account_id, server_state, expires_at) VALUES (?, ?, ?, ?)")
    .bind(loginId, account?.id ?? null, result.serverLoginState, Date.now() + LOGIN_TTL_MS)
    .run();
  if (Math.random() < 0.02) {
    await env.DB.prepare("DELETE FROM logins WHERE expires_at <= ?").bind(Date.now()).run();
  }
  return json({ loginId, loginResponse: result.loginResponse });
}

/// Login, step 2: verify the handshake and, on success, start a session.
async function opaqueLoginFinish(env: Env, request: Request): Promise<Response> {
  const body = await readJson<{ loginId?: string; finishLoginRequest?: string }>(request);
  if (!body?.loginId || !body?.finishLoginRequest) return json({ error: "invalid_request" }, 400);

  const row = await env.DB.prepare("SELECT account_id, server_state, expires_at FROM logins WHERE id = ?")
    .bind(body.loginId)
    .first<{ account_id: string | null; server_state: string; expires_at: number }>();
  if (!row || row.expires_at <= Date.now()) return json({ error: "invalid_credentials" }, 401);
  await env.DB.prepare("DELETE FROM logins WHERE id = ?").bind(body.loginId).run();

  try {
    finish_login(row.server_state, body.finishLoginRequest);
  } catch {
    return json({ error: "invalid_credentials" }, 401);
  }
  if (!row.account_id) return json({ error: "invalid_credentials" }, 401);

  const account = await env.DB.prepare("SELECT plan FROM accounts WHERE id = ?")
    .bind(row.account_id)
    .first<{ plan: string }>();
  const token = await startSession(env, row.account_id);
  return json({ token, plan: account?.plan ?? "active" });
}

/// Conversion: attach an OPAQUE record to an already-authenticated (legacy) account.
async function opaqueUpgradeStart(env: Env, account: AccountRow, request: Request): Promise<Response> {
  const body = await readJson<{ registrationRequest?: string }>(request);
  if (!body?.registrationRequest) return json({ error: "invalid_request" }, 400);
  const registrationResponse = create_registration_response(env.OPAQUE_SERVER_SETUP, account.email, body.registrationRequest);
  return json({ registrationResponse });
}

async function opaqueUpgradeFinish(env: Env, account: AccountRow, request: Request): Promise<Response> {
  const body = await readJson<{ registrationRecord?: string }>(request);
  if (!body?.registrationRecord) return json({ error: "invalid_request" }, 400);
  // A legacy account being converted to OPAQUE has not really changed its password; an account
  // that is already OPAQUE and registers again has. Only the latter gets a new timestamp.
  const before = await env.DB.prepare("SELECT scheme FROM accounts WHERE id = ?")
    .bind(account.id)
    .first<{ scheme: string }>();
  const changedAt = before?.scheme === "opaque" ? Date.now() : null;
  await env.DB.prepare(
    "UPDATE accounts SET opaque_record = ?, scheme = 'opaque', password_changed_at = COALESCE(?, password_changed_at) WHERE id = ?",
  )
    .bind(body.registrationRecord, changedAt, account.id)
    .run();
  return json({ ok: true, passwordChangedAt: changedAt ?? account.password_changed_at ?? null });
}

async function me(env: Env, account: AccountRow): Promise<Response> {
  return json({
    id: account.id,
    plan: account.plan,
    planValidUntil: account.plan_valid_until ?? null,
    passwordChangedAt: account.password_changed_at ?? null,
  });
}

/// Whether an account may write to the cloud. `comp` and `trialing` are active; `past_due` and
/// `canceled` stay writable until their paid period ends; everything else is read-only.
function hasActivePlan(account: AccountRow): boolean {
  if (account.plan === "active" || account.plan === "comp" || account.plan === "trialing") return true;
  if (account.plan === "past_due" || account.plan === "canceled") {
    return account.plan_valid_until != null && account.plan_valid_until > Date.now();
  }
  return false;
}

// ----- Billing (Paddle) -----
//
// The server is the only writer of `plan`: the client can start a checkout, but entitlement
// changes come exclusively from a signed webhook. Each endpoint depends only on what it needs —
// selling needs the API key + price, the webhook needs its secret — so a missing secret disables
// the webhook (and, with it, plan changes) instead of all of billing. See docs/BILLING.md.

const TRIAL_MS = 14 * 24 * 60 * 60 * 1000;

/// Whether the server can sell: an API key and a price. Missing the webhook secret does not turn
/// this off (it only disables the webhook that flips the plan).
function billingEnabled(env: Env): boolean {
  return Boolean(env.PADDLE_API_KEY && env.PADDLE_PRICE_ID);
}

/// Sandbox keys contain `_sdbx_`; everything else is production.
function paddleBase(env: Env): string {
  return (env.PADDLE_API_KEY ?? "").includes("_sdbx_") ? "https://sandbox-api.paddle.com" : "https://api.paddle.com";
}

/// The plan a brand-new account starts on: a 14-day trial once billing is set up (an API key and
/// a price exist), otherwise simply active (so a server without Paddle just works).
function initialPlan(env: Env): { plan: string; validUntil: number | null } {
  return billingEnabled(env) ? { plan: "trialing", validUntil: Date.now() + TRIAL_MS } : { plan: "active", validUntil: null };
}

/// Maps a Paddle subscription status and period to our entitlement.
function planFromSubscription(subscription: {
  status?: string;
  current_billing_period?: { ends_at?: string };
}): { plan: string; validUntil: number | null } {
  const ends = subscription.current_billing_period?.ends_at ? Date.parse(subscription.current_billing_period.ends_at) : NaN;
  const validUntil = Number.isFinite(ends) ? ends : null;
  switch (subscription.status) {
    case "active":
      return { plan: "active", validUntil };
    case "trialing":
      return { plan: "trialing", validUntil };
    case "past_due":
      return { plan: "past_due", validUntil };
    case "canceled":
      return { plan: "canceled", validUntil };
    default:
      return { plan: "expired", validUntil: Date.now() };
  }
}

/// Verifies Paddle's `Paddle-Signature: ts=…;h1=…` header over `${ts}:${rawBody}`.
async function verifyPaddleSignature(secret: string, header: string, body: string): Promise<boolean> {
  const parts: Record<string, string> = {};
  for (const pair of header.split(";")) {
    const index = pair.indexOf("=");
    if (index > 0) parts[pair.slice(0, index).trim()] = pair.slice(index + 1).trim();
  }
  const ts = parts.ts;
  const given = parts.h1;
  if (!ts || !given) return false;
  if (Math.abs(Date.now() / 1000 - Number(ts)) > 300) return false; // replay window
  const key = await crypto.subtle.importKey("raw", enc.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const digest = await crypto.subtle.sign("HMAC", key, enc.encode(`${ts}:${body}`));
  return timingSafeEqual(toHex(digest), given);
}

/// Applies a billing update to the account, matched by our account id (preferred) carried in
/// `custom_data`. Never trusts the client for entitlement — only the webhook calls this.
async function applyBilling(
  env: Env,
  accountId: string | undefined,
  customerId: string | undefined,
  subscriptionId: string | undefined,
  plan: { plan: string; validUntil: number | null } | undefined,
): Promise<void> {
  if (!accountId) return;
  await env.DB.prepare(
    `UPDATE accounts
        SET plan = COALESCE(?, plan),
            plan_valid_until = COALESCE(?, plan_valid_until),
            billing_provider = 'paddle',
            billing_customer_id = COALESCE(?, billing_customer_id),
            billing_subscription_id = COALESCE(?, billing_subscription_id)
      WHERE id = ?`,
  )
    .bind(plan?.plan ?? null, plan?.validUntil ?? null, customerId ?? null, subscriptionId ?? null, accountId)
    .run();
}

/// Creates a Paddle checkout bound to this account and returns its hosted URL.
async function createCheckout(env: Env, account: AccountRow): Promise<Response> {
  if (!billingEnabled(env)) return json({ error: "billing_unavailable" }, 503);
  const response = await fetch(`${paddleBase(env)}/transactions`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${env.PADDLE_API_KEY}`,
      "content-type": "application/json",
    },
    body: JSON.stringify({
      items: [{ price_id: env.PADDLE_PRICE_ID, quantity: 1 }],
      custom_data: { accountId: account.id },
      collection_mode: "automatic",
    }),
  });
  const body = (await response.json().catch(() => ({}))) as {
    data?: { checkout?: { url?: string } };
    error?: { code?: string; detail?: string };
  };
  const url = body.data?.checkout?.url;
  if (!response.ok || !url) {
    return json(
      { error: "checkout_failed", paddleStatus: response.status, paddleCode: body.error?.code ?? null },
      502,
    );
  }
  return json({ url });
}

/// Cancels at the end of the paid period (access continues until then).
async function cancelSubscription(env: Env, account: AccountRow): Promise<Response> {
  if (!env.PADDLE_API_KEY) return json({ error: "billing_unavailable" }, 503);
  const subscriptionId = account.billing_subscription_id;
  if (!subscriptionId) return json({ error: "no_subscription" }, 400);
  const response = await fetch(`${paddleBase(env)}/subscriptions/${subscriptionId}/cancel`, {
    method: "POST",
    headers: { authorization: `Bearer ${env.PADDLE_API_KEY}`, "content-type": "application/json" },
    body: JSON.stringify({ effective_from: "next_billing_period" }),
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => ({}))) as { error?: { code?: string } };
    return json({ error: "cancel_failed", paddleStatus: response.status, paddleCode: body.error?.code ?? null }, 502);
  }
  return json({ ok: true });
}

/// Paddle webhook: verify, dedupe, then flip the plan. Never called by the client.
async function paddleWebhook(env: Env, request: Request): Promise<Response> {
  if (!env.PADDLE_WEBHOOK_SECRET) return json({ error: "billing_unavailable" }, 503);
  const raw = await request.text();
  const signature = request.headers.get("paddle-signature") ?? "";
  if (!(await verifyPaddleSignature(env.PADDLE_WEBHOOK_SECRET, signature, raw))) {
    return json({ error: "invalid_signature" }, 401);
  }

  let event: {
    event_id?: string;
    event_type?: string;
    data?: {
      id?: string;
      status?: string;
      customer_id?: string;
      subscription_id?: string;
      action?: string;
      current_billing_period?: { ends_at?: string };
      custom_data?: { accountId?: string };
    };
  };
  try {
    event = JSON.parse(raw);
  } catch {
    return json({ error: "invalid_json" }, 400);
  }
  const eventId = event.event_id;
  if (!eventId) return json({ error: "invalid_event" }, 400);

  // Idempotency: retries of the same event must be a no-op.
  const recorded = await env.DB.prepare("INSERT OR IGNORE INTO billing_events (id, received_at) VALUES (?, ?)")
    .bind(eventId, Date.now())
    .run();
  if ((recorded.meta.changes ?? 0) === 0) return json({ ok: true, duplicate: true });

  const type = event.event_type ?? "";
  const data = event.data ?? {};

  if (type.startsWith("subscription.")) {
    await applyBilling(env, data.custom_data?.accountId, data.customer_id, data.id, planFromSubscription(data));
  } else if (type === "transaction.completed") {
    // The subscription events set the plan; this records the customer id meanwhile.
    await applyBilling(env, data.custom_data?.accountId, data.customer_id, undefined, undefined);
  } else if ((type === "adjustment.created" || type === "adjustment.updated") && data.subscription_id) {
    if ((data.action === "refund" || data.action === "chargeback") && data.status === "approved") {
      await env.DB.prepare("UPDATE accounts SET plan = 'expired', plan_valid_until = ? WHERE billing_subscription_id = ?")
        .bind(Date.now(), data.subscription_id)
        .run();
    }
  }

  return json({ ok: true });
}

async function listVaults(env: Env, account: AccountRow): Promise<Response> {
  const { results } = await env.DB.prepare(
    "SELECT id, name, name_cipher, vault_uuid, version, updated_at FROM vaults WHERE account_id = ? ORDER BY updated_at DESC",
  )
    .bind(account.id)
    .all();
  return json({ vaults: results ?? [] });
}

async function createVault(env: Env, account: AccountRow, request: Request): Promise<Response> {
  const body = await readJson<{ vaultUuid?: string; nameCipher?: string }>(request);
  const vaultUuid = body?.vaultUuid?.slice(0, 100) || null;
  const nameCipher = body?.nameCipher?.slice(0, 5000) || null;

  // The vault's own id is its identity: reuse the existing cloud copy instead of making
  // a second one for the same vault.
  if (vaultUuid) {
    const existing = await env.DB.prepare(
      "SELECT id, version FROM vaults WHERE account_id = ? AND vault_uuid = ?",
    )
      .bind(account.id, vaultUuid)
      .first<{ id: string; version: number }>();
    if (existing) return json({ id: existing.id, version: existing.version });
  }

  const id = crypto.randomUUID();
  await env.DB.prepare(
    "INSERT INTO vaults (id, account_id, name, name_cipher, vault_uuid, version, updated_at) VALUES (?, ?, '', ?, ?, 0, ?)",
  )
    .bind(id, account.id, nameCipher, vaultUuid, Date.now())
    .run();
  return json({ id, version: 0 });
}

/// Loads a vault row owned by the account, or null.
async function ownedVault(env: Env, account: AccountRow, id: string) {
  return env.DB.prepare(
    "SELECT id, name, version FROM vaults WHERE id = ? AND account_id = ?",
  )
    .bind(id, account.id)
    .first<{ id: string; name: string; version: number }>();
}

/// Updates only a vault's encrypted label (its name). The server still cannot read it, and any
/// legacy plaintext label left by older versions is cleared at the same time.
async function patchVaultLabel(env: Env, account: AccountRow, id: string, request: Request): Promise<Response> {
  const vault = await ownedVault(env, account, id);
  if (!vault) return json({ error: "not_found" }, 404);
  const body = await readJson<{ nameCipher?: string }>(request);
  if (!body?.nameCipher) return json({ error: "invalid_request" }, 400);
  await env.DB.prepare("UPDATE vaults SET name_cipher = ?, name = '', updated_at = ? WHERE id = ? AND account_id = ?")
    .bind(body.nameCipher.slice(0, 5000), Date.now(), id, account.id)
    .run();
  return json({ ok: true });
}

function blobKey(vaultId: string, version: number): string {
  return `vaults/${vaultId}/v${version}`;
}

async function getVault(env: Env, account: AccountRow, id: string): Promise<Response> {
  const vault = await ownedVault(env, account, id);
  if (!vault) return json({ error: "not_found" }, 404);
  if (vault.version === 0) return json({ version: 0, blob: null });

  const object = await env.BLOBS.get(blobKey(id, vault.version));
  if (!object) return json({ error: "blob_missing" }, 500);
  const bytes = new Uint8Array(await object.arrayBuffer());
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return json({ version: vault.version, blob: btoa(binary) });
}

/// Stores a new version if `If-Match` matches the current one (optimistic concurrency).
async function putVault(env: Env, account: AccountRow, id: string, request: Request): Promise<Response> {
  const vault = await ownedVault(env, account, id);
  if (!vault) return json({ error: "not_found" }, 404);
  if (!(await allow(env, `put:${account.id}`, 240, 900))) return json({ error: "rate_limited" }, 429);

  const ifMatch = request.headers.get("if-match");
  const base = ifMatch === null ? vault.version : Number(ifMatch);
  if (base !== vault.version) return json({ error: "conflict", version: vault.version }, 409);

  const bytes = new Uint8Array(await request.arrayBuffer());
  if (bytes.byteLength === 0) return json({ error: "empty" }, 400);
  if (bytes.byteLength > 8 * 1024 * 1024) return json({ error: "too_large" }, 413);

  const used = await env.DB.prepare(
    "SELECT COALESCE(SUM(size), 0) AS total FROM vaults WHERE account_id = ? AND id != ?",
  )
    .bind(account.id, id)
    .first<{ total: number }>();
  if ((used?.total ?? 0) + bytes.byteLength > ACCOUNT_QUOTA_BYTES) {
    return json({ error: "quota_exceeded" }, 413);
  }

  const next = vault.version + 1;
  // Immutable, versioned blob first (an orphan is harmless), then the atomic version bump.
  await env.BLOBS.put(blobKey(id, next), bytes);
  const result = await env.DB.prepare(
    "UPDATE vaults SET version = ?, updated_at = ?, size = ? WHERE id = ? AND version = ?",
  )
    .bind(next, Date.now(), bytes.byteLength, id, vault.version)
    .run();
  const changed = result.meta.changes ?? 0;
  if (changed === 0) return json({ error: "conflict", version: vault.version }, 409);

  // Keep only the current blob: the client already merged, so older versions are dead weight.
  // This bounds storage to one blob per vault no matter how many edits a user makes.
  if (vault.version > 0) {
    await env.BLOBS.delete(blobKey(id, vault.version));
  }
  return json({ version: next });
}

async function deleteVault(env: Env, account: AccountRow, id: string): Promise<Response> {
  const vault = await ownedVault(env, account, id);
  if (!vault) return json({ error: "not_found" }, 404);
  // Remove the stored blobs too, so nothing is left behind in R2.
  const listed = await env.BLOBS.list({ prefix: `vaults/${id}/` });
  if (listed.objects.length > 0) {
    await env.BLOBS.delete(listed.objects.map((object) => object.key));
  }
  await env.DB.prepare("DELETE FROM vaults WHERE id = ? AND account_id = ?")
    .bind(id, account.id)
    .run();
  return json({ ok: true });
}

/// Deletes the account, its vault rows, its session tokens and its stored blobs.
async function deleteAccount(env: Env, account: AccountRow): Promise<Response> {
  const { results } = await env.DB.prepare("SELECT id FROM vaults WHERE account_id = ?")
    .bind(account.id)
    .all<{ id: string }>();
  for (const vault of results ?? []) {
    const listed = await env.BLOBS.list({ prefix: `vaults/${vault.id}/` });
    if (listed.objects.length > 0) {
      await env.BLOBS.delete(listed.objects.map((object) => object.key));
    }
  }
  await env.DB.prepare("DELETE FROM vaults WHERE account_id = ?").bind(account.id).run();
  await env.DB.prepare("DELETE FROM sessions WHERE account_id = ?").bind(account.id).run();
  await env.DB.prepare("DELETE FROM accounts WHERE id = ?").bind(account.id).run();
  return json({ ok: true });
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const path = url.pathname.replace(/\/+$/, "") || "/";
    const method = request.method.toUpperCase();

    try {
      if (path === "/auth/register" && method === "POST") return await register(env, request);
      if (path === "/auth/login" && method === "POST") return await login(env, request);
      if (path === "/auth/register/start" && method === "POST") return await opaqueRegisterStart(env, request);
      if (path === "/auth/register/finish" && method === "POST") return await opaqueRegisterFinish(env, request);
      if (path === "/auth/login/start" && method === "POST") return await opaqueLoginStart(env, request);
      if (path === "/auth/login/finish" && method === "POST") return await opaqueLoginFinish(env, request);
      if (path === "/health") return json({ ok: true });
      if (path === "/billing/webhook" && method === "POST") return await paddleWebhook(env, request);

      const account = await authenticate(env, request);
      if (!account) return json({ error: "unauthorized" }, 401);

      if (path === "/auth/upgrade/start" && method === "POST") return await opaqueUpgradeStart(env, account, request);
      if (path === "/auth/upgrade/finish" && method === "POST") return await opaqueUpgradeFinish(env, account, request);

      if (path === "/account" && method === "DELETE") return await deleteAccount(env, account);

      // "Sign out everywhere": revoke every session except the one making this request.
      if (path === "/sessions" && method === "DELETE") {
        const token = bearerToken(request);
        const currentHash = token ? await sha256Hex(token) : "";
        await env.DB.prepare("DELETE FROM sessions WHERE account_id = ? AND token_hash != ?")
          .bind(account.id, currentHash)
          .run();
        return json({ ok: true });
      }

      // Billing: the client may start a checkout or cancel, but entitlement itself is only ever
      // changed by the webhook. These stay available when the plan has lapsed, so a user can renew.
      if (path === "/billing/checkout" && method === "POST") return await createCheckout(env, account);
      if (path === "/billing/cancel" && method === "POST") return await cancelSubscription(env, account);

      // `/me` and every read stay available even when the plan lapsed, so the app can show the
      // status and the user can always download — and delete — their own cloud data.
      if (path === "/me" && method === "GET") return await me(env, account);
      if (path === "/vaults" && method === "GET") return await listVaults(env, account);

      const match = path.match(/^\/vaults\/([^/]+)$/);
      if (match) {
        const id = match[1];
        if (method === "GET") return await getVault(env, account, id);
        if (method === "DELETE") return await deleteVault(env, account, id);
        if (method === "PATCH") return await patchVaultLabel(env, account, id, request);
      }

      // Writing new encrypted data needs an active plan.
      if (!hasActivePlan(account)) return json({ error: "payment_required", plan: account.plan }, 402);

      if (path === "/vaults" && method === "POST") return await createVault(env, account, request);
      if (match && method === "PUT") return await putVault(env, account, match[1], request);

      return json({ error: "not_found" }, 404);
    } catch (error) {
      return json({ error: "server_error", message: String(error) }, 500);
    }
  },
};
