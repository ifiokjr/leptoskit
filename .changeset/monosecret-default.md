---
"release": minor
---

# Use Monosecret for development secrets

New projects declare their database and sign-in secret names in `monosecret.toml`. `leptoskit dev` resolves that manifest through Monosecret before starting, with `--no-secrets` for hosts that supply the environment. Existing projects without a manifest keep their current startup. The secret management guide recommends Monosecret for server and deployment credentials; client bundles gain no dependencies.
