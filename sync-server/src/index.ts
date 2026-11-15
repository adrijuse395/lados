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
  const token = bearerToken(request);
  if (!token) return null;
  const tokenHash = await sha256Hex(token);
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
  await env.DB.prepare(
    "INSERT INTO accounts (id, email, salt, verifier, scheme, plan, created_at) VALUES (?, ?, ?, ?, 'argon2v2', 'active', ?)",
  )
    .bind(id, email, salt, verifier, Date.now())
    .run();

  const token = await startSession(env, id);
  return json({ token, plan: "active" });
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

async function me(env: Env, account: AccountRow): Promise<Response> {
  return json({ id: account.id, plan: account.plan });
}

async function listVaults(env: Env, account: AccountRow): Promise<Response> {
  const { results } = await env.DB.prepare(
    "SELECT id, name, vault_uuid, version, updated_at FROM vaults WHERE account_id = ? ORDER BY name",
  )
    .bind(account.id)
    .all();
  return json({ vaults: results ?? [] });
}

async function createVault(env: Env, account: AccountRow, request: Request): Promise<Response> {
  const body = await readJson<{ name?: string; vaultUuid?: string }>(request);
  const name = (body?.name || "Vault").slice(0, 200);
  const vaultUuid = body?.vaultUuid?.slice(0, 100) || null;

  // The vault's own id is its identity: reuse the existing cloud copy instead of making
  // a second one for the same vault.
  if (vaultUuid) {
    const existing = await env.DB.prepare(
      "SELECT id, name, version FROM vaults WHERE account_id = ? AND vault_uuid = ?",
    )
      .bind(account.id, vaultUuid)
      .first<{ id: string; name: string; version: number }>();
    if (existing) return json({ id: existing.id, name: existing.name, version: existing.version });
  }

  const id = crypto.randomUUID();
  await env.DB.prepare(
    "INSERT INTO vaults (id, account_id, name, vault_uuid, version, updated_at) VALUES (?, ?, ?, ?, 0, ?)",
  )
    .bind(id, account.id, name, vaultUuid, Date.now())
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
      if (path === "/health") return json({ ok: true });

      const account = await authenticate(env, request);
      if (!account) return json({ error: "unauthorized" }, 401);

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
