---
"release": minor
---

# Live queries over the RPC

`leptoskit_rpc` gains the live hub: a subscription runs a query as its caller, records the read set that produced the result, and re-runs only when a commit could have changed it.

- **`live` feature** (enabled by `postgres` and `sqlite`). `LiveHub` and `LiveConnection` are usable without a socket; the server's `router` adds `GET /live`, and `Server::start` runs the change poll loop.
- **Protocol v1** (`leptoskit.live.v1.json`): `hello`, `sub`, `unsub`, `mutate` and `ping` from the client; `welcome`, `transition`, `done`, `auth_status`, `reconnect` and `pong` from the server. A mutation's result carries the commit version, so a client can wait for read-your-writes.
- **Read sets decide the work.** `ReadEntry::affected_by` answers in three-valued logic; a subscription is re-run only when a change could have affected it. A keyed list is sent as a `Patch` of removals and upserts.
- **Policies everywhere.** Every re-run uses the subscriber's identity and context, so a client never sees another person's rows. Tests cover fan-out to two connections, a write outside the read set, unsubscribe, and a policy that hides one person's rows from another's transition.
- **One-shot operations keep working.** `#[rpc::query]` registers both the JSON handler and a query handler that also returns its read set; mutations keep their serializable retries and receipt.
