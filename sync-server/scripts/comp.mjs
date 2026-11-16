// Grants or revokes complimentary (free, unlimited) Lados Sync access — no payment involved.
//
//   ADMIN_TOKEN=… node scripts/comp.mjs list
//   ADMIN_TOKEN=… node scripts/comp.mjs grant  friend@example.com "family"
//   ADMIN_TOKEN=… node scripts/comp.mjs revoke friend@example.com
//
// ADMIN_TOKEN is the Worker secret of the same name. SYNC_SERVER overrides the server URL
// (useful for a self-hosted instance).
//
// The server never has, and never needs, anyone's password. A granted email either already has an
// account (upgraded to free access at once) or gets it automatically when that person registers
// in the app — they then sign in with their own master password, exactly like anyone else.

const BASE = (process.env.SYNC_SERVER || "https://lados-sync.ladospwm.workers.dev").replace(/\/+$/, "");
const token = process.env.ADMIN_TOKEN;
if (!token) {
  console.error("ADMIN_TOKEN is not set (it is the Worker secret of the same name).");
  process.exit(1);
}

async function request(method, body) {
  const response = await fetch(`${BASE}/admin/comp`, {
    method,
    headers: { authorization: `Bearer ${token}`, ...(body ? { "content-type": "application/json" } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await response.text();
  let json = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    /* not JSON */
  }
  if (response.status === 404) {
    console.error("The server has no /admin/comp route — set the ADMIN_TOKEN secret and redeploy.");
    process.exit(1);
  }
  if (!response.ok) {
    console.error(`Request failed (${response.status}):`, json ?? text);
    process.exit(1);
  }
  return json;
}

const [command, email, note] = process.argv.slice(2);

if (command === "list") {
  const { grants } = await request("GET");
  if (!grants.length) {
    console.log("No complimentary accounts.");
  } else {
    for (const grant of grants) console.log(`${grant.email}${grant.note ? `  — ${grant.note}` : ""}`);
  }
} else if (command === "grant") {
  if (!email) {
    console.error("usage: comp.mjs grant <email> [note]");
    process.exit(1);
  }
  const result = await request("POST", { email, note: note ?? null });
  console.log(
    result.accountFound
      ? `Granted free access to ${email} (existing account upgraded now).`
      : `Granted free access to ${email}. It applies the moment that person registers.`,
  );
} else if (command === "revoke") {
  if (!email) {
    console.error("usage: comp.mjs revoke <email>");
    process.exit(1);
  }
  const result = await request("DELETE", { email });
  console.log(
    result.accountUpdated
      ? `Revoked free access for ${email}; the account is now read-only until it subscribes.`
      : `Revoked free access for ${email}.`,
  );
} else {
  console.error("usage: comp.mjs <grant|revoke|list> [email] [note]");
  process.exit(1);
}
