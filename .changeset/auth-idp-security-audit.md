---
"release": patch
---

# Harden sign-in, merges, passwords, and the provider

`leptoskit_auth`:

- Starting a passkey or wallet sign-in no longer cancels everyone else's. These sign-ins share one subject before anyone is known, so a script that kept starting them could break sign-in for every visitor.
- An account that is disabled or locked can no longer be merged into another. Before, merging let a banned account shed its ban, and let someone take guesses at a locked account's second factor.
- Setting or changing a password counts against the account's password limit. A password reset counts against the per-IP limit before it asks the breach service. Argon2 runs at most two hashes per core at once.
- A refresh token traded again within the 30-second grace works only once. If a thief makes that trade, the app's own token ends the session when it is next used, and `refresh_reused` is recorded.

`leptoskit_idp`:

- Scope lists are de-duplicated in linear time; before, a long list cost quadratic time.
- Every authorization parameter is capped at 4 KiB, so pushed requests can't store megabytes.
- A DPoP proof or client assertion with an extreme `iat` or `exp` is refused instead of overflowing.
- A code redeemed by two requests at once ends the tokens the winning request got, as a later replay already did.
