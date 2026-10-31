---
"release": minor
---

# Call a provider's API for the person who signed in

A provider configured with `store_tokens = true` keeps its access and refresh tokens, encrypted with `AUTH_ENCRYPTION_KEYS`, so your server can call that provider's API on the person's behalf:

```rust
if let Some(github) = auth.provider_access_token(me.account, "github").await? {
	client.get("https://api.github.com/user/repos").bearer_auth(&github.access_token);
}
```

- It's off by default; signing in alone never keeps tokens.
- An access token about to expire is refreshed first. Sign-in asks for a refresh token where the provider needs asking, such as Google's `access_type=offline`.
- Disconnecting the provider, merging its account into another, or the new `Auth::delete_account` deletes the tokens and revokes them at providers that allow it, including Apple, which asks apps to when someone deletes their account.
- `leptoskit auth rotate-keys` re-encrypts them, and says how many.
