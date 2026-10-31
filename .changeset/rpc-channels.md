---
"release": minor
---

# Channels and presence

`leptoskit_rpc` gains typed channels for traffic that should be seen but not stored: typing indicators, cursors, presence.

- **`#[rpc::channel]`** turns a join argument struct into a channel with a message type, a presence type, and an `authorize` function. The check runs as the connection's identity when it joins, and again when that identity changes.
- **`Live`** keeps each channel instance's members, relays signals to everyone else at most once, and sends the full presence list on every join and leave. A disconnect removes the member; a reconnect rejoins every channel.
- **`rpc::use_channel`** joins while a component is mounted, leaves when its arguments change or it unmounts, and exposes `members()`, the last 100 `signals()`, and `signal(…)`. Without a socket it returns empty signals, so screens render during SSR.
- **Docs.** "Real-time screens" gains a channels and presence section.
