// The billing funnel, read from D1. Read-only.
//
//   node scripts/funnel.mjs
//
// `trial_used_at` records when an account began its trial, so "trialled → paid" is a real cohort
// number. `plan` alone can never answer it: it only ever holds the current state of an account.
//
// wrangler is invoked through its JavaScript entry point, so nothing passes through a shell and
// SQL containing quotes or percent signs is safe. It uses the wrangler login you already have.

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";

const DATABASE = process.env.LADOS_DB || "lados-sync";
const WRANGLER = join(process.cwd(), "node_modules", "wrangler", "bin", "wrangler.js");

if (!existsSync(WRANGLER)) {
  console.error(`wrangler is not installed at ${WRANGLER} — run npm install in sync-server first`);
  process.exit(1);
}

/** Runs one read-only query and returns its rows. */
function rows(sql) {
  const result = spawnSync(
    process.execPath,
    [WRANGLER, "d1", "execute", DATABASE, "--remote", "--json", "--command", sql],
    { encoding: "utf8" },
  );
  if (result.status !== 0) {
    console.error((result.stderr || result.stdout || `wrangler exited ${result.status}`).trim());
    process.exit(1);
  }
  // wrangler writes its banner to stderr; keep only the JSON on stdout.
  const start = result.stdout.indexOf("[");
  const parsed = JSON.parse(start >= 0 ? result.stdout.slice(start) : "[]");
  return parsed[0]?.results ?? [];
}

const count = (sql) => Number(Object.values(rows(sql)[0] ?? {})[0] ?? 0);

const accounts = count("SELECT COUNT(*) AS n FROM accounts");
const trials = count("SELECT COUNT(*) AS n FROM accounts WHERE trial_used_at IS NOT NULL");
const trialledAndPaid = count(
  "SELECT COUNT(*) AS n FROM accounts WHERE trial_used_at IS NOT NULL AND billing_subscription_id IS NOT NULL",
);
const subscribed = count("SELECT COUNT(*) AS n FROM accounts WHERE billing_subscription_id IS NOT NULL");
const paidWithoutTrial = count(
  "SELECT COUNT(*) AS n FROM accounts WHERE billing_subscription_id IS NOT NULL AND trial_used_at IS NULL",
);
const comp = count("SELECT COUNT(*) AS n FROM accounts WHERE plan = 'comp'");
const grants = count("SELECT COUNT(*) AS n FROM comp_grants");
const events = count("SELECT COUNT(*) AS n FROM billing_events");
const vaults = count("SELECT COUNT(*) AS n FROM vaults");

const plans = rows("SELECT plan, COUNT(*) AS n FROM accounts GROUP BY plan ORDER BY n DESC");
const kinds = rows(
  "SELECT CASE WHEN email LIKE '%@example.com' OR email LIKE '%@ladospwm.%' THEN 'test' ELSE 'real' END AS kind, COUNT(*) AS n FROM accounts GROUP BY kind ORDER BY kind",
);
const neverPaid = rows(
  "SELECT plan, COUNT(*) AS n FROM accounts WHERE billing_subscription_id IS NULL AND plan <> 'comp' GROUP BY plan ORDER BY n DESC",
);

const rate =
  trials === 0 ? "no trials yet" : `${trialledAndPaid}/${trials}  (${((100 * trialledAndPaid) / trials).toFixed(1)}%)`;

console.log(`Lados Sync — billing funnel  (${DATABASE}, remote)\n`);
console.log(`  accounts            ${accounts}`);
console.log(`  trials started      ${trials}`);
console.log(`  trialled -> paid    ${rate}`);
console.log(`  ever subscribed     ${subscribed}${paidWithoutTrial ? `   (${paidWithoutTrial} of them without a trial)` : ""}`);
console.log(`  comp accounts       ${comp}   (pending grants: ${grants})`);
console.log(`  vaults              ${vaults}`);
console.log(`  webhook events      ${events}`);
if (plans.length) console.log(`\n  by plan             ${plans.map((r) => `${r.plan} ${r.n}`).join(", ")}`);
if (kinds.length) console.log(`  accounts            ${kinds.map((r) => `${r.kind} ${r.n}`).join(", ")}`);
if (neverPaid.length) console.log(`  never paid          ${neverPaid.map((r) => `${r.plan} ${r.n}`).join(", ")}`);

if (trials === 0) {
  console.log(
    "\n  No trial has ever run, so there is nothing to convert yet. Billing is live and every new\n" +
      "  account starts one; this line fills in once a shipped build reaches real people.",
  );
}
