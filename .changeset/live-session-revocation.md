---
"leptoskit_auth": patch
"leptoskit_rpc": patch
---

# End live connections when their sessions end

First-party session revocation, account disabling, and session expiry now end open WebSocket and SSE connections, even when they have no subscriptions. The shared hub watches authentication reads, and validates them on the primary. Bearer clients receive an expiry warning and can renew in place. Custom identity adapters can return an `IdentityLease` with their own dependencies and deadlines.

A refused re-authentication closes the previous identity. Changing identity discards queued updates from the old identity, and slow actions cannot return the old user's result to the new user. The app client drops resume state and fails pending calls when credentials are refused.

The auth pack's accounts and sessions tables now capture live changes. Generate and apply a database migration when upgrading an existing app.
