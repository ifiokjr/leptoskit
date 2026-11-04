# Verifying a release

Anyone using your app has to trust that the code running it is the code you published. When your repository is public, leptoskit lets them check that for themselves: the backend files you release can be compared byte for byte with an inventory you published, so a reviewer, or a coding agent they point at the bundle, reads exactly the files you released.

This page covers the backend. Each check says exactly what it proves and what it leaves out.

## Checking a backend bundle

Before comparing or reviewing a backend release, check that its source files, dependency sources, build declaration, infrastructure definitions and executable match the inventory you trust:

```sh
leptoskit verify check \
  --bundle ./backend-bundle \
  --manifest ./backend-inventory.json \
  --sha256 <trusted-inventory-sha256>
```

This is an offline file check for a prepared backend bundle, not a connection to a running backend. It applies to backend artifacts used by any client platform. It does not verify a web or native client. Unix file permission bits are compared when available; checking an inventory carrying Unix permissions requires a host and extraction method that preserve those permissions.

Obtain the expected SHA-256 from independently authenticated release evidence or a previously trusted record. A checksum supplied alongside an untrusted download cannot establish who produced it. A successful check means the local files match that inventory; it does not prove that the executable was built from the included sources, that dependencies are complete, that a server runs it, or that its code is safe.

### Preparing an inventory

Prepare a directory containing only public materials:

```text
backend-bundle/
  source/
    Cargo.toml
    Cargo.lock
    src/main.rs
  dependencies/
  infra/
  artifacts/
    backend
  build.json
  secrets.json
```

Include the complete application source in `source/`, dependency sources in `dependencies/`, infrastructure definitions in `infra/`, and the built backend executable at `artifacts/backend`. The dependency and infrastructure directories must exist, even when empty. Include other build inputs and runtime libraries under their relevant directories; the checker inventories the supplied files but does not discover missing inputs or run Cargo.

`build.json` declares the intended recipe. All fields below are required, including empty lists and `false`:

```json
{
	"repository": "https://github.com/example/backend",
	"commit": "1234567890123456789012345678901234567890",
	"builder_image": "registry.example/builder@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	"target": "x86_64-unknown-linux-gnu",
	"package": "backend",
	"binary": "backend",
	"features": [],
	"no_default_features": false,
	"rustflags": []
}
```

Replace the illustrative repository, full commit ID and builder digest with your release's values. The checker validates the declaration's format but does not contact the repository or builder, execute this recipe, or confirm that it produced the executable. Unknown recipe fields are rejected.

`secrets.json` contains names only:

```json
["DATABASE_URL", "API_KEY"]
```

Use `[]` when the backend needs no secrets. Do not include assignments or secret values. The command does not read your environment or collect working-tree files, but it cannot identify secrets hardcoded in files you supply.

Create the inventory outside the bundle:

```sh
leptoskit verify record \
  --bundle ./backend-bundle \
  --out ./backend-inventory.json
```

The command prints the inventory's SHA-256 and refuses to overwrite an existing inventory. It records file paths, lengths, hashes, Unix file permissions where supported, and directory entries. Missing, changed and extra entries—including empty directories—fail a later check. Symbolic links and special files are rejected. Directory permissions, ownership, timestamps, ACLs and extended attributes are not checked. Use an extracted directory that is not being modified while checking it.

The inventory is unsigned. Authenticate it separately before others rely on it. Both commands state explicitly that build provenance and the live deployment were not verified.
