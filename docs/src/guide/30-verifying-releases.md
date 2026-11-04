# Verifying a release

Anyone using your app has to trust that the code running it is the code you published. When your repository is public, leptoskit lets them check that for themselves. You build your backend from a commit on GitHub with `leptoskit verify build`; anyone else runs `leptoskit verify reproduce` to fetch that commit, rebuild it, and compare the result with your executable byte for byte. A reviewer, or a coding agent they point at your repository, then knows the code they read is the code you shipped.

This page covers backend executables built for Linux, and the websites and APIs that serve them. The commands run on macOS and Linux; building and reproducing need Docker or Podman. They prove what a release contains and which origins you approve to serve it; they do not prove which executable a live server is running, and they say nothing about whether the code is safe.

## Building a backend people can verify

Your repository must be public on GitHub, with `Cargo.lock` committed. Write a recipe naming the commit and how to build it:

```json
{
	"repository": "https://github.com/example/backend",
	"commit": "1234567890123456789012345678901234567890",
	"builder_image": "docker.io/library/rust@sha256:ed2c95ca377d6571b124b05e75e01207973ded639dce7d5542eb7bcd797e32aa",
	"target": "aarch64-unknown-linux-gnu",
	"package": "backend",
	"binary": "server",
	"features": [],
	"no_default_features": false,
	"rustflags": [],
	"infra": "deploy"
}
```

Use your own repository, the full 40-character commit ID, and your package and binary names. Every field except `infra` is required, including empty lists and `false`, so nothing is left to a default:

- `builder_image` is the container that compiles your code, pinned by its digest. The example is the official Rust 1.99.1 image. A tag such as `rust:latest` is refused, because it can point to a different compiler tomorrow. If your repository has a `rust-toolchain.toml`, the image must already contain that toolchain: nothing is downloaded while compiling.
- `target` is `x86_64-unknown-linux-gnu` or `aarch64-unknown-linux-gnu` (or their `-musl` variants), and must match your machine's processor, since the compiler runs natively in the container. Use an x86-64 machine for an x86-64 server.
- `features` and `no_default_features` are passed to Cargo as written. `rustflags` is the complete list of compiler flags: rustflags in your repository's Cargo configuration are not used.
- `infra` optionally names a directory in the commit, such as your deployment configuration, to publish alongside the source.

Then build:

```sh
leptoskit verify build \
  --recipe recipe.json \
  --secret DATABASE_URL --secret API_KEY \
  --out release
```

The command:

1. fetches the commit from your public repository without credentials (pass `--checkout <clone>` to read it from a local clone instead), and writes its exact files, executable bits included, to `release/bundle/source/`. Submodules and symbolic links are refused, so every source is in the commit;
2. downloads every dependency `Cargo.lock` names into `release/bundle/dependencies/`. This is the only step with network access, and your source is read-only during it;
3. compiles the binary twice in fresh containers with networking disabled, the source and dependencies read-only, and a fixed environment, then requires the two executables to be identical. A difference fails the build, because nobody else could reproduce it;
4. writes `release/inventory.json`, which pins every file in the bundle, and `release/provenance.json`, a standard [SLSA provenance](https://slsa.dev/spec/v1.0/provenance) statement naming the commit, builder image, lockfile, compiler version, and executable digest.

`--secret` records the names of the environment variables your server reads secrets from, in `bundle/secrets.json`; their values are never read. The command prints the executable's and the inventory's SHA-256. Publish the bundle and inventory with your release, and publish the inventory SHA-256 somewhere people can authenticate it, such as your release notes or a signed tag. The files themselves are not signed.

## Reproducing a release

Anyone with the bundle, its inventory, and the inventory SHA-256 from a source they trust can check the release:

```sh
leptoskit verify reproduce \
  --bundle release/bundle \
  --manifest release/inventory.json \
  --sha256 <trusted-inventory-sha256>
```

Each line of the output is one claim that held:

```text
bundle matches the pinned inventory (4238 entries)
source/ and infra/ are exactly https://github.com/example/backend at commit 1234567…
dependencies/ is exactly what source/Cargo.lock resolves to
artifacts/backend was rebuilt byte for byte from source/ and dependencies/ in docker.io/library/rust@sha256:…
Not verified: which executable a live server runs, or whether the code is safe.
```

The command fetches the commit from the repository named in the recipe, downloads the dependencies the lockfile names, and compiles the binary once with networking disabled. Any difference fails it and names the first file that differs: changed source, a modified dependency, or an executable that the published source does not produce.

`--checkout <clone>` compares the source with a local clone instead of fetching it, and says so in its output. `--offline` skips both network steps and only rebuilds the executable from the bundle's own files; the skipped claims are reported as not checked. Rebuilding needs a machine with the same processor architecture as the recipe's target.

## Proving which websites are yours

Your source is public, so anyone can deploy a copy of it, or copy your website, and claim to be you. A deployment policy says which origins serve your deployment and which releases they may run, signed with a key only you hold. A copy at another address fails the check, a forged policy fails the signature, and an old policy cannot replace a newer one.

Create a key once, and publish its public half in your repository, for example in your readme or `SECURITY.md`, so people get it from your code rather than from a website:

```sh
leptoskit verify keygen --out deployment.key
```

Keep `deployment.key` out of your repository. Write a policy listing every origin and the releases `verify build` produced:

```json
{
	"format_version": 1,
	"kind": "leptoskit-deployment-policy",
	"repository": "https://github.com/example/app",
	"deployment": "production",
	"origins": ["https://example.com", "https://api.example.com"],
	"sequence": 1,
	"issued_at": 1793750400,
	"expires_at": 1796342400,
	"releases": [
		{
			"commit": "1234567890123456789012345678901234567890",
			"inventory_sha256": "<the inventory SHA-256 verify build printed>",
			"executable_sha256": "<the executable SHA-256 verify build printed>"
		}
	]
}
```

Times are Unix seconds. Increase `sequence` every time you sign a new policy, and keep the expiry short, a month or so, so a policy you have replaced stops working on its own. Origins are written exactly as `https://host`, with a port only when it isn't 443. Sign it and serve the result at `/.well-known/leptoskit/deployment.json` on every origin it lists:

```sh
leptoskit verify sign --policy policy.json --key deployment.key --out deployment.json
```

Anyone can then check an origin against the key from your repository:

```sh
leptoskit verify deployment \
  --origin https://example.com \
  --key <public key from the repository> \
  --repository https://github.com/example/app \
  --state example-production.json
```

The check fetches the policy over HTTPS and refuses it unless your key signed it, it names that repository and origin, and it has not expired. `--state` remembers the newest policy seen, so a later check refuses an older or conflicting one. The output lists the approved releases with their inventory SHA-256, which is the authenticated digest `verify reproduce` needs.

A policy is your signed statement about what your origins serve. It does not prove what the server at an origin is actually running: that needs hardware attestation of the server, which leptoskit does not provide yet.

## Checking a backend bundle

`leptoskit verify check` is the first step of `reproduce` on its own: it needs no container or network. Before comparing or reviewing a backend release, check that its source files, dependency sources, build declaration, infrastructure definitions and executable match the inventory you trust:

```sh
leptoskit verify check \
  --bundle ./backend-bundle \
  --manifest ./backend-inventory.json \
  --sha256 <trusted-inventory-sha256>
```

This is an offline file check for a prepared backend bundle, not a connection to a running backend. It applies to backend artifacts used by any client platform. It does not verify a web or native client. Unix file permission bits are compared when available; checking an inventory carrying Unix permissions requires a host and extraction method that preserve those permissions.

Obtain the expected SHA-256 from independently authenticated release evidence or a previously trusted record. A checksum supplied alongside an untrusted download cannot establish who produced it. A successful check means the local files match that inventory; it does not prove that the executable was built from the included sources, that dependencies are complete, that a server runs it, or that its code is safe.

### Preparing an inventory by hand

`verify build` writes the bundle and its inventory for you. To inventory a backend built some other way, prepare a directory containing only public materials:

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

Replace the illustrative repository, full commit ID and builder digest with your release's values. `record` and `check` validate the declaration's format but do not contact the repository or builder, execute this recipe, or confirm that it produced the executable; `reproduce` does. Unknown recipe fields are rejected.

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
