# Red-team prompts

Hand these to the agents running the pre-release security assessment described in
[`docs/RED-TEAM.md`](../docs/RED-TEAM.md):

- [`PROMPT-explorer.md`](./PROMPT-explorer.md) — a relentless *user*: bugs, edge cases, robustness.
- [`PROMPT-attacker.md`](./PROMPT-attacker.md) — an authorized *attacker*: authorization, payment,
  account takeover.

Each prompt is **self-contained**, but both assume the staging environment is already up (see
`docs/RED-TEAM.md` §4). The placeholders they contain — `<STAGING_URL>`, `<TEST_ACCOUNT_EMAIL>`,
`<TEST_ACCOUNT_PASSWORD>`, … — are filled in when the task is handed over.

Put the real values in `local.md` (git-ignored) and paste them into the prompt you send. **Never
commit the staging URL or the test credentials.**
