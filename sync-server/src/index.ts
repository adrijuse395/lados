// Lados Sync — minimal, zero-knowledge sync API.
//
// The server stores an OPAQUE, encrypted copy of each vault and never sees
// plaintext. It authenticates accounts and tracks versions for optimistic
// concurrency. All merging happens on the client (see docs/SYNC.md in the app repo).
//
// SPDX-License-Identifier: AGPL-3.0-only

export interface Env {
  DB: D1Database;
  BLOBS: R2Bucket;
  TOKEN_TTL_SECONDS: string;
}

const enc = new TextEncoder();

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
  plan: string;
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
  const header = request.headers.get("authorization") || "";
  const match = header.match(/^Bearer\s+(.+)$/i);
  if (!match) return null;
  const tokenHash = await sha256Hex(match[1]);
  const row = await env.DB.prepare(
    `SELECT a.id AS id, a.plan AS plan
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

async function register(env: Env, request: Request): Promise<Response> {
  const body = await readJson<{ email?: string; authSecret?: string }>(request);
  const email = body?.email?.trim().toLowerCase();
  const authSecret = body?.authSecret;
  if (!email || !email.includes("@") || !authSecret) {
    return json({ error: "invalid_request" }, 400);
  }

  const existing = await env.DB.prepare("SELECT id FROM accounts WHERE email = ?")
    .bind(email)
    .first();
  if (existing) return json({ error: "email_taken" }, 409);

  const salt = randomHex(16);
  const verifier = await verifierFor(salt, authSecret);
  const id = crypto.randomUUID();
  await env.DB.prepare(
    "INSERT INTO accounts (id, email, salt, verifier, scheme, plan, created_at) VALUES (?, ?, ?, ?, 'argon2', 'active', ?)",
  )
    .bind(id, email, salt, verifier, Date.now())
    .run();

  const token = await startSession(env, id);
  return json({ token, plan: "active" });
}

async function login(env: Env, request: Request): Promise<Response> {
  const body = await readJson<{ email?: string; authSecret?: string; legacyAuthSecret?: string }>(request);
  const email = body?.email?.trim().toLowerCase();
  const authSecret = body?.authSecret;
  const legacyAuthSecret = body?.legacyAuthSecret;
  if (!email || !authSecret) return json({ error: "invalid_request" }, 400);

  const row = await env.DB.prepare(
    "SELECT id, salt, verifier, scheme, plan FROM accounts WHERE email = ?",
  )
    .bind(email)
    .first<{ id: string; salt: string; verifier: string; scheme: string; plan: string }>();
  if (!row) return json({ error: "invalid_credentials" }, 401);

  if (row.scheme === "argon2") {
    const candidate = await verifierFor(row.salt, authSecret);
    if (!timingSafeEqual(candidate, row.verifier)) {
      return json({ error: "invalid_credentials" }, 401);
    }
  } else {
    // Account created before the auth-secret hardening: verify with the old derivation, then
    // replace the stored credential with the Argon2 one so the account is upgraded for good.
    if (!legacyAuthSecret) return json({ error: "invalid_credentials" }, 401);
    const candidate = await verifierFor(row.salt, legacyAuthSecret);
    if (!timingSafeEqual(candidate, row.verifier)) {
      return json({ error: "invalid_credentials" }, 401);
    }
    const salt = randomHex(16);
    const verifier = await verifierFor(salt, authSecret);
    await env.DB.prepare(
      "UPDATE accounts SET salt = ?, verifier = ?, scheme = 'argon2' WHERE id = ?",
    )
      .bind(salt, verifier, row.id)
      .run();
  }

  const token = await startSession(env, row.id);
  return json({ token, plan: row.plan });
}

async function me(env: Env, account: AccountRow): Promise<Response> {
  return json({ id: account.id, plan: account.plan });
}

async function listVaults(env: Env, account: AccountRow): Promise<Response> {
  const { results } = await env.DB.prepare(
    "SELECT id, name, version, updated_at FROM vaults WHERE account_id = ? ORDER BY name",
  )
    .bind(account.id)
    .all();
  return json({ vaults: results ?? [] });
}

async function createVault(env: Env, account: AccountRow, request: Request): Promise<Response> {
  const body = await readJson<{ name?: string }>(request);
  const name = (body?.name || "Vault").slice(0, 200);
  const id = crypto.randomUUID();
  await env.DB.prepare(
    "INSERT INTO vaults (id, account_id, name, version, updated_at) VALUES (?, ?, ?, 0, ?)",
  )
    .bind(id, account.id, name, Date.now())
    .run();
  return json({ id, name, version: 0 });
}

/// Loads a vault row owned by the account, or null.
async function ownedVault(env: Env, account: AccountRow, id: string) {
  return env.DB.prepare(
    "SELECT id, name, version FROM vaults WHERE id = ? AND account_id = ?",
  )
    .bind(id, account.id)
    .first<{ id: string; name: string; version: number }>();
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

  const ifMatch = request.headers.get("if-match");
  const base = ifMatch === null ? vault.version : Number(ifMatch);
  if (base !== vault.version) return json({ error: "conflict", version: vault.version }, 409);

  const bytes = new Uint8Array(await request.arrayBuffer());
  if (bytes.byteLength === 0) return json({ error: "empty" }, 400);
  if (bytes.byteLength > 8 * 1024 * 1024) return json({ error: "too_large" }, 413);

  const next = vault.version + 1;
  // Immutable, versioned blob first (an orphan is harmless), then the atomic version bump.
  await env.BLOBS.put(blobKey(id, next), bytes);
  const result = await env.DB.prepare(
    "UPDATE vaults SET version = ?, updated_at = ? WHERE id = ? AND version = ?",
  )
    .bind(next, Date.now(), id, vault.version)
    .run();
  const changed = result.meta.changes ?? 0;
  if (changed === 0) return json({ error: "conflict", version: vault.version }, 409);

  return json({ version: next });
}

async function deleteVault(env: Env, account: AccountRow, id: string): Promise<Response> {
  const vault = await ownedVault(env, account, id);
  if (!vault) return json({ error: "not_found" }, 404);
  await env.DB.prepare("DELETE FROM vaults WHERE id = ? AND account_id = ?")
    .bind(id, account.id)
    .run();
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
      if (path === "/health") return json({ ok: true });

      const account = await authenticate(env, request);
      if (!account) return json({ error: "unauthorized" }, 401);

      if (account.plan !== "active") return json({ error: "payment_required", plan: account.plan }, 402);

      if (path === "/me" && method === "GET") return await me(env, account);
      if (path === "/vaults" && method === "GET") return await listVaults(env, account);
      if (path === "/vaults" && method === "POST") return await createVault(env, account, request);

      const match = path.match(/^\/vaults\/([^/]+)$/);
      if (match) {
        const id = match[1];
        if (method === "GET") return await getVault(env, account, id);
        if (method === "PUT") return await putVault(env, account, id, request);
        if (method === "DELETE") return await deleteVault(env, account, id);
      }

      return json({ error: "not_found" }, 404);
    } catch (error) {
      return json({ error: "server_error", message: String(error) }, 500);
    }
  },
};
