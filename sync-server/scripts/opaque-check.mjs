// Sanity check for @serenity-kit/opaque (the server's OPAQUE, WASM build of opaque-ke).
// Runs a full registration + login in Node, and prints a fresh server setup.
//
//   node scripts/opaque-check.mjs

import { ready, client, server } from "@serenity-kit/opaque";

await ready;

const serverSetup = server.createSetup();
const password = "correct horse battery staple";
const userIdentifier = "user@example.com";

const registration = client.startRegistration({ password });
const registrationResponse = server.createRegistrationResponse({
  serverSetup,
  userIdentifier,
  registrationRequest: registration.registrationRequest,
});
const registrationFinished = client.finishRegistration({
  password,
  registrationResponse: registrationResponse.registrationResponse,
  clientRegistrationState: registration.clientRegistrationState,
});
const registrationRecord = registrationFinished.registrationRecord;

const login = client.startLogin({ password });
const serverLogin = server.startLogin({
  serverSetup,
  registrationRecord,
  startLoginRequest: login.startLoginRequest,
  userIdentifier,
});
const loginFinished = client.finishLogin({
  password,
  loginResponse: serverLogin.loginResponse,
  clientLoginState: login.clientLoginState,
});
const serverLoginFinished = server.finishLogin({
  serverLoginState: serverLogin.serverLoginState,
  finishLoginRequest: loginFinished.finishLoginRequest,
});

console.log("registration record length:", registrationRecord.length);
console.log("session keys match:", loginFinished.sessionKey === serverLoginFinished.sessionKey);
console.log("server setup:", serverSetup);
