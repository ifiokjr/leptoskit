---
leptoskit_cli: major
---

# leptoskit_cli: the command line

`leptoskit_cli` is the `leptoskit` and `lk` command: scaffold a project with `new`, run every platform at once with `dev`, then `build`, `serve`, and `test`. It also writes routes, translations, and database migrations, generates sign-in screens and plugins, ships code push releases, uploads to the app and extension stores with `publish`, drives devices and simulators (also over MCP), and lints and formats the code.

Deployment lives here too: `infra` manages OpenTofu infrastructure, `deploy` releases a commit or rolls back, `console` hosts or connects to the self-hosted deployment service, or pairs a worker with the hosted LeptosKit app, and `verify` reproduces backend builds and checks signed deployment policies. New projects resolve their development secrets through Monosecret, and `leptoskit dev --no-secrets` uses the existing environment. The binary is a thin wrapper over the library, so tooling can drive the same commands directly.
