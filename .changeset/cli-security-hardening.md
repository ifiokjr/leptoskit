---
"release": patch
---

# Harden the CLI against web pages and leaked secrets

- `leptoskit mcp http` now refuses requests a web page could send: any request with an `Origin` or `Sec-Fetch-Site` header, a `POST` whose content type isn't `application/json`, and a `Host` other than the address it listens on. Before, a site open in your browser could post to it and run the `control` tool. Send `content-type: application/json` with every message. Binding it to a non-loopback `--address` now prints a warning, since it has no authentication.
- The Firefox source archive leaves out symlinks that point outside the project, and `.jks`, `.keystore`, and `.pfx` files, so `leptoskit publish firefox` can't upload a linked credential or an Android keystore.
- A malformed publish credentials file reports the line that is wrong, without quoting it, so the secret on that line isn't printed.
- `leptoskit publish macos` names its `.app`, `.zip`, and `.dmg` from the app's name with path separators replaced, so a name like `AC/DC` stays in the output folder instead of naming a path elsewhere.
- Branch database names no longer panic when a long app name has a multi-byte character where the name is cut.
- The release workflow `leptoskit new` writes now reads the default branch from the event (`$GITHUB_DEFAULT_BRANCH` was never set, so the check failed every tagged release), and checks that manual runs start from a commit on the default branch too.
