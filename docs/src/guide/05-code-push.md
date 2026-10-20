# Code push

An app shell that went through store review is mostly the same every release: the runtime, the native widgets, the permissions. What changes is the interface, and on native platforms the interface is a wasm bundle. Code push ships that bundle over the air, so an interface change reaches devices without another store review.

## The release flow

```bash
leptoskit codepush keygen --out ~/.config/leptoskit/codepush.key
leptoskit build --release --platform native
leptoskit codepush release --channel production --signing-key ~/.config/leptoskit/codepush.key
leptoskit codepush serve
```

1. **`keygen`** creates your publisher key, once. It writes the secret key to a file only you can read and prints the public key for `codepush.public_keys`. The public key ships inside the app shell.
2. **`release`** packs a built directory into a release: a single-file container with a content-hashed manifest, signed with your publisher key. It also computes a binary delta from the channel's previous release.
3. **`serve`** runs the release registry over HTTP, under `/api/codepush`.
4. **Devices check on launch.** The [`CodePushClient`](https://docs.rs/leptoskit_codepush) asks the registry for anything newer than what it runs and checks the offer's signature before downloading anything. It then downloads a delta when one applies to its exact current release (otherwise the full container), verifies every hash, and stages the update for the next launch.

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

### `leptoskit codepush serve`

| Flag            | Default    | Meaning                                           |
| --------------- | ---------- | ------------------------------------------------- |
| `--store <DIR>` | `releases` | The release store.                                |
| `--dir <DIR>`   | `dist`     | Static files served next to the registry.         |
| `--port <PORT>` | `4100`     | The port. The address comes from `serve.address`. |

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

- The offer has to carry an ed25519 signature, made by a key in the client's trusted set, over the manifest hash. The manifest names the app, channel, version, build, the mandatory flag, and every file's hash, so the signature covers all of them. This is checked before anything is downloaded.
- The offer has to be for the client's own app and channel.
- The offer has to be newer than the release the client runs, comparing version and then build. An old release stays validly signed forever, so this is what stops a registry from rolling devices back or replaying a release they already have.
- The container's manifest hash has to match the signed offer, and every file has to match its hash in the manifest.
- Each file has to decompress to exactly its declared size, which stops decompression bombs.
- Manifest paths have to be relative and stay inside the bundle. Absolute paths and `..` are rejected before anything touches the disk.
- A delta's output size is checked before any memory is allocated, and capped at 512 MiB.
- If a delta can't be fetched, doesn't parse, or doesn't apply, the client falls back to the full download.

A rejected offer is an error, never a silent "up to date", and nothing is staged.

### What signing doesn't cover

The registry can still withhold updates: it can answer "up to date" or serve nothing at all, and the client can't tell that apart from a quiet week. Serve the registry over HTTPS so a network attacker can't do the same.

The client compares offers against the release you tell it is running. Pass the release the shell actually loads, whether that's the bundle from the store build or the last staged update. Otherwise an older signed release can look new.

Anyone who holds the secret key can ship code to your users. Keep it in a secret store, not in the repository.

### Rotating keys

`TrustedKeys` holds a set of keys, so a rotation never strands devices:

1. Generate the new key and add its public key to `codepush.public_keys` next to the old one.
2. Ship that shell through the store and wait for it to reach your users.
3. Sign releases with the new key.
4. Remove the old public key in a later store release.

## Delivering updates safely

A staged update takes effect on the next launch. To roll back, release the previous build again under a new build number. Keep anything that needs new native capabilities, such as new entitlements or new custom widget kinds, in a store release, and ship everything else through code push.
