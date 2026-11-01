---
"release": patch
---

# A release tag must name a reviewed commit; the live socket has a budget

The scaffold's release workflow checked a pushed tag against `app.version` read from the tagged tree itself, so a tag over any commit passed its own check and reached every store credential and a write token. The workflow now checks out the whole history and refuses a tag whose commit the default branch does not carry, and drops the upload keystore and the unlocked keychain when publishing ends. The report it appends to the run summary escapes the text the stores and the project sent, so nothing can mark up the page maintainers read.

The live WebSocket ran every frame it was handed: subscriptions cycled at line rate ran queries at line rate, and mutations ran unbounded per socket. A connection now has a frame budget — a burst that covers a reconnecting client's whole subscription set, refilling every second — so a client that sends faster than the server serves is closed with a reason. Token guessing was closed off by refusing-and-disconnecting (#482), which a frame budget now also bounds per handshake.

A live query could miss a write it should have seen. The read-set matcher treated a change-log image it couldn't decode — raw SQL put a shape in a column the build doesn't expect, an `infinity` timestamp, schema drift during a rolling deploy — as an image that matched nothing, so subscribers kept stale results indefinitely, including rows a policy change should have hidden. An undecodable image now counts as a change, which re-runs the read.

Also: a job idempotency key taken by a job its caller can't see is answered as taken instead of as not found; a devtools agent can no longer wedge the hub's session lock with a tree patch whose parents form a cycle, and the hub keeps at most 256 commands per agent instead of every one forever; the interface refuses to build editor links for paths that climb out of the project; and the audit log's reader documents that its rule narrows the database layer only, not raw SQL.
