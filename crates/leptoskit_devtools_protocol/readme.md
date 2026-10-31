# leptoskit_devtools_protocol

The serde types shared by leptoskit's DevTools agents, hub, interface, and MCP tools. It has no platform code: agents on the web, native hosts, and guests all speak the same messages.

Every message carries [`PROTOCOL_VERSION`]. A hub refuses a session that speaks another version and names both versions in the error.
