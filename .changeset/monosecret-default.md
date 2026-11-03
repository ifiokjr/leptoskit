---
"release": minor
---

# Use Monosecret for development secrets

New projects declare their database and sign-in secret names in `monosecret.toml`. `leptoskit dev` resolves that manifest through Monosecret before starting, with `--no-secrets` for hosts that supply the environment. Existing projects without a manifest keep their current startup. The secret management guide recommends Monosecret for server and deployment credentials.

`leptoskit::secrets` exposes Monosecret 0.5's small value-access client on native and Wasm targets. Native servers can read named environment values; Wasm hosts provide values explicitly. Default provider and runtime features are disabled, keeping the resolver, network clients, and provider dependencies out of application bundles.
