## What and why

<!-- One paragraph. Link the issue: "Closes #123". -->

## Claims

<!-- Labels are law (spec §2). List every claim this PR adds or changes. Delete the table if there are none. -->

| Claim | Label | Evidence |
|---|---|---|
|  |  |  |

- [ ] No label is upgraded without evidence; loopback and one-machine runs are `OBSERVED (LOCAL)`
- [ ] `docs/TARGETS.md`, `docs/FRONTIER.md` and `apps/README.md` are updated if a claim changed

## Checks

- [ ] Green gate passes locally (fmt, clippy `-D warnings`, tests; see CONTRIBUTING.md)
- [ ] C ABI changes are append-only and bump the ABI minor
- [ ] App / SDK builds touched by this PR pass
- [ ] No tokens, keys or pairing codes in logs or evidence (EVD-006)
