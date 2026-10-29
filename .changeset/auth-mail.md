---
"release": minor
---

# Sign-in email and `[auth]` configuration

Sign-in is configured in `leptoskit.toml` under `[auth]`. Secrets aren't written there: each setting that needs one names the environment variable that holds it. `Auth::from_config` builds the service from that configuration, and `LeptoskitConfig::from_toml` lets a server embed its configuration, so it needs no file at runtime.

Email goes out over SMTP (`auth-smtp`), or as a JSON webhook signed per Standard Webhooks (`auth-webhook`) that your endpoint passes to any provider. With neither configured, development keeps email in the mailbox at `/_leptoskit/mail`, and a release build refuses to start.
