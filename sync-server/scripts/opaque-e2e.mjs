// End-to-end OPAQUE auth against the deployed worker.
//
// Uses the @serenity-kit/opaque client (the same opaque-ke as the Rust app) against our
// own `opaque-wasm` server build, so this is a real cross-implementation check.
//
//   node scripts/opaque-e2e.mjs

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

async function call(method, path, body, token) {
  const headers = {};
  if (token) headers.authorization = `Bearer ${token}`;
  if (body !== undefined) headers["content-type"] = "application/json";
  const response = await fetch(`${BASE}${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  const text = await response.text();
  let json = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    /* not JSON */
  }
  return { status: response.status, json };
}

const rid = () => Math.random().toString(36).slice(2, 10);

await ready;
console.log(`Testing OPAQUE at ${BASE}\n`);

const email = `opaque-${rid()}@example.com`;
const password = "correct horse battery staple";

// Registration
const registration = client.startRegistration({ password });
let r = await call("POST", "/auth/register/start", {
  email,
  registrationRequest: registration.registrationRequest,
});
check("register/start returns a response", r.status === 200 && typeof r.json?.registrationResponse === "string", `status ${r.status}`);

const registrationFinished = client.finishRegistration({
  password,
  registrationResponse: r.json.registrationResponse,
  clientRegistrationState: registration.clientRegistrationState,
});
r = await call("POST", "/auth/register/finish", { email, registrationRecord: registrationFinished.registrationRecord });
const token = r.json?.token;
check("register/finish returns a session token", r.status === 200 && typeof token === "string", `status ${r.status}`);

// Registration is refused for an existing email
r = await call("POST", "/auth/register/start", { email, registrationRequest: registration.registrationRequest });
check("register/start for an existing email is refused (409)", r.status === 409);

// Login
const login = client.startLogin({ password });
r = await call("POST", "/auth/login/start", { email, startLoginRequest: login.startLoginRequest });
const { loginId, loginResponse } = r.json ?? {};
check("login/start returns a loginId and response", r.status === 200 && !!loginId && !!loginResponse);

const loginFinished = client.finishLogin({
  password,
  loginResponse,
  clientLoginState: login.clientLoginState,
});
check("client completes the handshake", loginFinished !== undefined);

r = await call("POST", "/auth/login/finish", { loginId, finishLoginRequest: loginFinished.finishLoginRequest });
check("login/finish returns a session token", r.status === 200 && typeof r.json?.token === "string", `status ${r.status}`);

r = await call("GET", "/me", undefined, r.json?.token);
check("the new session works", r.status === 200 && ["active", "trialing", "comp"].includes(r.json?.plan));

// A loginId cannot be reused
r = await call("POST", "/auth/login/finish", { loginId, finishLoginRequest: loginFinished.finishLoginRequest });
check("a used loginId is refused (401)", r.status === 401);

// A wrong password fails the handshake (the client detects it, so we send junk)
const bad = client.startLogin({ password: "definitely wrong password" });
r = await call("POST", "/auth/login/start", { email, startLoginRequest: bad.startLoginRequest });
const badFinished = client.finishLogin({ password: "definitely wrong password", loginResponse: r.json.loginResponse, clientLoginState: bad.clientLoginState });
check("a wrong password does not authenticate on the client", badFinished === undefined);

// An unregistered email gets a dummy response (no enumeration), and cannot finish
const ghost = client.startLogin({ password });
r = await call("POST", "/auth/login/start", { email: `ghost-${rid()}@example.com`, startLoginRequest: ghost.startLoginRequest });
check("an unknown email still gets a login response (no enumeration)", r.status === 200 && !!r.json?.loginResponse);
const ghostFinished = client.finishLogin({ password, loginResponse: r.json.loginResponse, clientLoginState: ghost.clientLoginState });
if (ghostFinished) {
  const ghostResult = await call("POST", "/auth/login/finish", { loginId: r.json.loginId, finishLoginRequest: ghostFinished.finishLoginRequest });
  check("an unknown email cannot finish (401)", ghostResult.status === 401);
} else {
  check("an unknown email cannot finish (client detects)", true);
}

// Clean up
r = await call("DELETE", "/account", undefined, token);
check("clean up the account", r.status === 200);

console.log(`\n${pass} passed, ${failures.length} failed`);
if (failures.length > 0) {
  console.log(`Failures: ${failures.join(", ")}`);
  process.exit(1);
}
