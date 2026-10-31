---
"release": minor
---

# `leptoskit publish` and `leptoskit skill`

**`leptoskit publish`** uploads a browser extension to the Chrome Web Store (API v2, with a service account), Microsoft Edge Add-ons (the Publish API), and addons.mozilla.org (API v5, with the source archive). Stores are named in `[publish]`.

- An upload is a draft that reaches nobody. `--submit` sends it for review, after a confirmation in a terminal, and `--dry-run` builds and checks without contacting a store.
- Every run ends with **What to do next**: each step the stores' APIs can't take, with the page to open, the text to paste from the project's `store/` folder, and whether an agent or the account holder must do it. The steps are also written to `.leptoskit/publish/next-steps.md` and `next-steps.json`.
- `leptoskit publish credentials` lists which credentials are set and where to make the missing ones. Credentials come from the environment or a `--credentials` file outside the project, and are never printed.

**`leptoskit skill`** prints leptoskit's skill for coding agents, matching the CLI's version, and `leptoskit skill install` installs it for Claude Code and other agents. Its publishing guide walks an agent through a release, including the dashboard steps with computer use, and stops for every step that belongs to the account holder.
