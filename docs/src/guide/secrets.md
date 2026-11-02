# Secret management

Database passwords, sign-in keys, and service credentials need a home outside your source code. Monosecret is leptoskit's default and recommended secret manager. It resolves the names in `monosecret.toml` from your environment, OS keyring, or a team provider before starting your server.

New projects include a manifest. Install [Monosecret](https://monosecret.dev), then start development as usual:

```sh
leptoskit dev
```

The CLI runs development through `monosecret run`. Resolution failures stop startup; they never fall back to an unconfigured server. Existing projects keep their current behavior until you add `monosecret.toml`. Use `leptoskit dev --no-secrets` when your deployment host already supplies the environment.

## Declare the names

For an existing project, add this file beside `leptoskit.toml`:

```toml
[project]
name = "my_app"
revision = "1.0"

[defaults]
providers = ["env", "keyring"]

[profiles.default]
DATABASE_URL = { description = "Postgres connection URL", required = true }
AUTH_ENCRYPTION_KEYS = { description = "Sign-in encryption keys", required = true }
```

Commit this declaration. Store the values in Monosecret's keyring or configure your team's provider in the manifest. Do not put credentials in `default` fields. Generated SQLite projects use a local database URL as a non-secret default; Postgres development can still use leptoskit's local database setup when no URL is stored.

The names must match the corresponding `_env` fields in `leptoskit.toml`. When adding sign-in providers, mail, read replicas, or publishing credentials, declare their environment names here too. The auth provider command lists the names it needs.

For database commands, wrap the command explicitly:

```sh
monosecret run -- leptoskit db migrate
```

For a production server, resolve the production profile before launching the binary:

```sh
monosecret run --profile production -- ./target/release/my_app
```

Declare `[profiles.production]` with the names and provider configuration for your deployment. Required values fail before the server starts. A platform that already injects secrets can run the binary directly.

## What reaches each platform

The same server-side workflow applies to web, iOS, Android, macOS, Windows, Linux, terminal, and experimental HarmonyOS apps. Their clients call the server; database credentials and sign-in encryption keys stay on that server. Monosecret adds no dependency to their application or Wasm bundles. Builds do not invoke secret resolution.

For tokens acquired after someone signs in, use the [secure-storage plugin](../platforms/plugins.md#reactive-settings-and-secrets) on supported native hosts. Browser applications cannot hide a value from the person running the browser. Give clients only credentials they are authorized to hold, such as their own session tokens.
