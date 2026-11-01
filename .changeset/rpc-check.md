---
"release": minor
---

# `leptoskit rpc check`

`leptoskit rpc check --against rpc.json` compares the current published RPC with a recorded spec and fails on what would break an installed client: a removed function, a changed kind, route, or encoding, a changed result schema, a removed argument, a new required argument, or a new required scope. `--from` reads the current spec from a file or URL instead of building the server. The generated-client chapter explains the workflow.
