# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-11-01)

### Fixes

#### End live connections when their sessions end

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #482](https://github.com/ifiokjr/leptoskit/pull/482)

First-party session revocation, account disabling, and session expiry now end open WebSocket and SSE connections, even when they have no subscriptions. The shared hub watches authentication reads, and validates them on the primary. Bearer clients receive an expiry warning and can renew in place. Custom identity adapters can return an `IdentityLease` with their own dependencies and deadlines.

A refused re-authentication closes the previous identity. Changing identity discards queued updates from the old identity, and slow actions cannot return the old user's result to the new user. The app client drops resume state and fails pending calls when credentials are refused.

The auth pack's accounts and sessions tables now capture live changes. Generate and apply a database migration when upgrading an existing app.
