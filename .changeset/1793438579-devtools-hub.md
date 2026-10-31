---
"release": minor
---

# Add the DevTools hub

`leptoskit dev` starts a loopback hub unless `--no-devtools` is passed. Agents open sessions, post events, and long-poll for commands; front ends read the mirrored state and stream changes. `--devtools-port` changes the port, and `.leptoskit/devtools.json` records the URL for MCP and the panel. The hub refuses another protocol version, binds only loopback, and checks `Host`, `Origin`, `Sec-Fetch-Site`, and the JSON content type.
