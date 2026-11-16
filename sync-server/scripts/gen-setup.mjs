// Prints a fresh OPAQUE server setup to stdout (for `wrangler secret put`).
import { ready, server } from "@serenity-kit/opaque";

await ready;
process.stdout.write(server.createSetup());
