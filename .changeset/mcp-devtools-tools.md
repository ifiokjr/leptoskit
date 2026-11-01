---
"release": minor
---

# Debug a running app from `leptoskit mcp`

`leptoskit mcp` gains eight tools that read a running app through the devtools hub `leptoskit dev` starts: `devtools_sessions`, `inspect_tree` (with a depth and a subtree root), `inspect_node`, `explain_styles`, `query_cache`, `navigate`, `highlight` (on one session, or on every session of the app by match key), and `set_state`. They find the hub through `.leptoskit/devtools.json`, pick the only or newest session when there is no doubt, and list the sessions when there is. The leptoskit skill tells agents to use them, and "Controlling devices" documents them.
