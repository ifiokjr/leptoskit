# Code push

An app shell that went through store review is mostly the same every release: the runtime, the native widgets, the permissions. What changes is the interface, and on native platforms the interface is a wasm bundle. Code push ships that bundle over the air, so an interface change reaches devices without another store review.

## The release flow

```bash
leptoskit codepush keygen --out ~/.config/leptoskit/codepush.key
leptoskit native bundle --out dist
leptoskit codepush release --channel production --signing-key ~/.config/leptoskit/codepush.key
leptoskit codepush publish --endpoint https://s3.us-west-004.backblazeb2.com --bucket my-app-updates --region us-west-004
```

1. **`keygen`** creates your publisher key, once. It writes the secret key to a file only you can read and prints the public key for `codepush.public_keys`. The public key ships inside the app shell.
2. **`native bundle`** compiles the native guest to `dist/app.wasm` and writes `dist/runtime.json`, which names the native ABI, shell contract, and plugins the guest needs.
3. **`release`** packs a built directory into a release: a single-file container with a content-hashed manifest, signed with your publisher key. It also computes a binary delta from the channel's previous release.
4. **`publish`** uploads the channel to an S3-compatible bucket. A CDN in front of the bucket is the whole registry. If you'd rather run a server, `leptoskit codepush serve` serves the same files over HTTP under `/api/codepush`.
5. **Devices download, try, and confirm.** In the generated iOS and Android hosts, the user taps **Check for update**. The [`CodePushClient`](https://docs.rs/leptoskit_codepush) reads the channel's `index.json` and checks its signature before downloading anything. If the release is newer than what the device runs, it asks for the delta from its exact current release, downloads the full container if there isn't one, and verifies every hash. `InstalledBundles` then checks `runtime.json` against the installed shell and stores the signed container. **Activate update** runs it as a trial, and the release becomes the device's working release only after it renders native controls. See [native application projects](../platforms/host-projects.md#code-push) for the full lifecycle.

## Code push with no server

A release never changes once it's published, and every file in it is addressed by its version or by a hash. The only thing that changes is which release is newest. So the registry is a tree of static files that any object store or CDN can serve:

```text
<app>/<channel>/index.json                                 the newest release: its manifest and signature
<app>/<channel>/<version>+<build>/release.lkb              the container
<app>/<channel>/<version>+<build>/release.sig              the publisher signature
<app>/<channel>/<version>+<build>/deltas/<hash>.jsondelta  the delta from the container with that SHA-256
```

A device makes plain GET requests and nothing else:

1. `index.json`. A 404 means the channel has no releases yet.
2. `deltas/<sha256 of my container>.jsondelta`. A 404 means there's no delta from what the device runs, so it moves on.
3. `release.lkb`, only when there was no usable delta.

The release store on disk already has this layout. `release` writes `index.json` after the release's other files, and `publish` uploads every new release file before it touches the bucket's `index.json`. A device therefore never learns about a release whose files aren't there yet. If a publish is interrupted, the channel stays on its previous release, and running `publish` again picks up where it stopped.

The bucket doesn't need to be trusted. `index.json` carries the signed manifest, so the client rejects anything you didn't sign, before it downloads anything. A bucket or CDN can only withhold updates, or keep serving an old index, which the client treats as "up to date".

Point the client at the CDN's base URL, for example `https://updates.example.com`. If you used `--prefix`, include it: `https://updates.example.com/<prefix>`.

### Hosting

- **Backblaze B2 behind Cloudflare.** Create a public bucket, then proxy a Cloudflare hostname to the bucket's friendly URL. Egress from B2 to Cloudflare is free. Publish with `--endpoint https://s3.<region>.backblazeb2.com --region <region>`.
- **Cloudflare R2.** Connect a custom domain to the bucket. Publish with `--endpoint https://<account id>.r2.cloudflarestorage.com`, using the default `--region auto`.
- **AWS S3, behind CloudFront or not.** Publish with `--endpoint https://s3.<region>.amazonaws.com --region <region>`.
- **MinIO and other local servers.** Add `--path-style`.

Each object is uploaded with its `Content-Type` and `Cache-Control`. Release files are `immutable` and cached for a year. `index.json` is cached for 60 seconds, so that's about how long a new release takes to reach devices behind a CDN. Some CDNs, Cloudflare included, don't cache `.json` or `.lkb` files by default. Add a cache rule for the update hostname that respects the origin's `Cache-Control`.

A device that runs a container with no matching delta asks for one and gets a 404. If your CDN caches 404s, keep that short. Missing deltas never appear later anyway, because `publish` writes deltas before the index that names their release.

## Commands

### `leptoskit codepush keygen`

| Flag           | Default | Meaning                                                  |
| -------------- | ------- | -------------------------------------------------------- |
| `--out <PATH>` | —       | Where to write the secret key. An existing file is kept. |

The secret key is 64 hex characters. Keep it out of version control, and never paste it into logs or issues.

### `leptoskit codepush release`

| Flag                   | Default      | Meaning                                    |
| ---------------------- | ------------ | ------------------------------------------ |
| `--from <DIR>`         | `dist`       | The built directory to release.            |
| `--store <DIR>`        | `releases`   | The release store.                         |
| `--channel <NAME>`     | `production` | The channel to publish to.                 |
| `--mandatory`          | off          | Asks devices to install without prompting. |
| `--signing-key <PATH>` | —            | The file holding the publisher key.        |

Every release is signed. Without `--signing-key`, the key is read from the `LEPTOSKIT_CODEPUSH_SIGNING_KEY` environment variable, which suits a CI secret. When `codepush.public_keys` is set, releasing with a key that isn't in it is an error, since no app would install the result.

The release is versioned as `app.version+<build>`. The build number goes up by one for each release of the same version. Releasing a version older than the channel's latest is an error, and so is releasing a version and build that already exist. A published release never changes.

### `leptoskit codepush publish`

| Flag                | Default      | Meaning                                                 |
| ------------------- | ------------ | ------------------------------------------------------- |
| `--store <DIR>`     | `releases`   | The release store.                                      |
| `--channel <NAME>`  | `production` | The channel to publish.                                 |
| `--endpoint <URL>`  | —            | The S3 API endpoint of the bucket's provider.           |
| `--bucket <NAME>`   | —            | The bucket.                                             |
| `--region <REGION>` | `auto`       | The bucket's region. `auto` suits R2.                   |
| `--prefix <PATH>`   | —            | A key prefix, so the bucket can hold other things too.  |
| `--path-style`      | off          | Address the bucket as `<endpoint>/<bucket>`, for MinIO. |

Credentials come only from the environment: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and `AWS_SESSION_TOKEN` for temporary credentials. They're never printed. The key only needs to read and write objects in the bucket.

`publish` uploads every release newer than the one the bucket's `index.json` names, oldest first, then points the index at the store's newest release. It refuses to:

- roll the channel back, when the bucket already names a newer release than the store holds;
- replace a file, when the bucket has different contents for a release or file the store also has, because another store released the same version and build;
- publish an unsigned release.

`release` numbers builds and computes deltas from the local store, so the store needs the bucket's newest release before you release. On a machine that keeps `releases/`, that's already true. On a CI runner that starts empty, run `pull` first. Run one publisher per channel at a time.

### `leptoskit codepush pull`

Copies the channel's newest release from the bucket into the store. It takes the same flags and credentials as `publish`.

Before anything is written, the release has to be signed by a key in `codepush.public_keys`, and its container has to hash to the signed manifest. So a tampered bucket can't plant a release for your next build to number from or patch against. Only the newest release is pulled. That's all `release` needs: the build to count up from, and the container devices run, which the next delta is computed against. Pulling when the store already has the release changes nothing, and an empty bucket leaves the store empty.

A stateless CI job is then three commands:

```bash
leptoskit codepush pull --endpoint "$S3_ENDPOINT" --bucket my-app-updates --region us-west-004
leptoskit codepush release --channel production
leptoskit codepush publish --endpoint "$S3_ENDPOINT" --bucket my-app-updates --region us-west-004
```

The signing key comes from `LEPTOSKIT_CODEPUSH_SIGNING_KEY`, and the bucket credentials from `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`.

### `leptoskit codepush serve`

| Flag            | Default    | Meaning                                           |
| --------------- | ---------- | ------------------------------------------------- |
| `--store <DIR>` | `releases` | The release store.                                |
| `--dir <DIR>`   | `dist`     | Static files served next to the registry.         |
| `--port <PORT>` | `4100`     | The port. The address comes from `serve.address`. |

`serve` answers with the same files a published bucket holds, so devices can't tell the two apart. It sends `index.json` with `Cache-Control: no-cache`, which means a new release shows up on the next check. The older `check`, `download`, and `delta` endpoints still work too.

### `leptoskit codepush check`

Asks the store what a device on a given release would be offered:

| Flag                 | Default      | Meaning                           |
| -------------------- | ------------ | --------------------------------- |
| `--store <DIR>`      | `releases`   | The release store.                |
| `--channel <NAME>`   | `production` | The channel to check.             |
| `--version <SEMVER>` | `0.1.0`      | The version the device runs.      |
| `--build <N>`        | `1`          | The build number the device runs. |

## What the client checks

Everything a device downloads is untrusted until it has been verified:

- `index.json` has to carry an ed25519 signature, made by a key in the client's trusted set, over the manifest hash. The manifest names the app, channel, version, build, the mandatory flag, and every file's hash, so the signature covers all of them. This is checked before anything is downloaded.
- The release has to be for the client's own app and channel.
- The release has to be newer than the one the client runs, comparing version and then build. An old release stays validly signed forever, so this is what stops a registry from rolling devices back or replaying a release they already have. An index that isn't newer means "up to date". That's also what a device sees when a CDN still serves the index from before it updated.
- The container's manifest hash has to match the signed offer, and every file has to match its hash in the manifest.
- Each file has to decompress to exactly its declared size, which stops decompression bombs.
- Manifest paths have to be relative and stay inside the bundle. Absolute paths and `..` are rejected before anything touches the disk.
- A delta's output size is checked before any memory is allocated, and capped at 512 MiB.
- If a delta can't be fetched, doesn't parse, or doesn't apply, the client falls back to the full download.

A release that fails these checks is an error, never a silent "up to date", and nothing is staged.

Your platform's `Transport` has to report a 404 as `CodePushError::NotFound`. That's how the client tells "no releases yet" and "no delta from here" apart from a network failure.

### What signing doesn't cover

The registry, whether that's a server, a bucket, or a CDN, can still withhold updates: it can keep serving an old `index.json` or serve nothing at all, and the client can't tell that apart from a quiet week. Serve the registry over HTTPS so a network attacker can't do the same.

The client compares offers against the release you tell it is running. Pass the release the shell actually loads, whether that's the bundle from the store build or the last staged update. Otherwise an older signed release can look new.

Anyone who holds the secret key can ship code to your users. Keep it in a secret store, not in the repository.

### Rotating keys

`TrustedKeys` holds a set of keys, so a rotation never strands devices:

1. Generate the new key and add its public key to `codepush.public_keys` next to the old one.
2. Ship that shell through the store and wait for it to reach your users.
3. Sign releases with the new key.
4. Remove the old public key in a later store release.

## Delivering updates safely

An activated update is a trial. If it traps, the host rejects it, keeps the working release on screen, and refuses that release from then on. A trial that never renders is never confirmed, so like a trial cut short by the app being killed, it is rejected on the next launch. This only covers startup: a release that renders and then misbehaves stays installed. To roll that back, release the previous build again under a new build number.

Keep anything that needs new native capabilities, such as new plugins, entitlements, or custom widget kinds, in a store release, and change `native.update_contract` in that release. Bundles whose `runtime.json` names a different ABI, contract, or a plugin the shell lacks are refused before they are stored. Interpreted code is a plausible fit for the App Store and Google Play rules on downloaded code, but it is not blanket approval: an update that changes what the app is for can still need review.
