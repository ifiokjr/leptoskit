# Code push

An app shell that went through store review is mostly the same every release: the runtime, the native widgets, the permissions. What changes is the interface, and on native platforms the interface is a wasm bundle. Code push ships that bundle over the air, so an interface change reaches devices without another store review.

## The release flow

```bash
leptoskit build --release --platform native
leptoskit codepush release --channel production
leptoskit codepush serve
```

1. **`release`** packs a built directory into a release: a single-file container with a content-hashed manifest. It also computes a binary delta from the channel's previous release.
2. **`serve`** runs the release registry over HTTP, under `/api/codepush`.
3. **Devices check on launch.** The [`CodePushClient`](https://docs.rs/leptoskit_codepush) asks the registry for anything newer than what it runs, downloads a delta when one applies to its exact current release (otherwise the full container), verifies every hash, and stages the update for the next launch.

## Commands

### `leptoskit codepush release`

| Flag               | Default      | Meaning                                    |
| ------------------ | ------------ | ------------------------------------------ |
| `--from <DIR>`     | `dist`       | The built directory to release.            |
| `--store <DIR>`    | `releases`   | The release store.                         |
| `--channel <NAME>` | `production` | The channel to publish to.                 |
| `--mandatory`      | off          | Asks devices to install without prompting. |

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

- The container's manifest hash has to match the offer, and every file has to match its hash in the manifest.
- Each file has to decompress to exactly its declared size, which stops decompression bombs.
- Manifest paths have to be relative and stay inside the bundle. Absolute paths and `..` are rejected before anything touches the disk.
- A delta's output size is checked before any memory is allocated, and capped at 512 MiB.
- If a delta can't be fetched, doesn't parse, or doesn't apply, the client falls back to the full download.

> **Warning:** Releases are not signed yet. Hash checks prove the bytes arrived intact, but not who published them. Serve your registry over HTTPS only, and treat anyone who can write to it as able to ship code to your users.

## Delivering updates safely

A staged update takes effect on the next launch. To roll back, release the previous build again under a new build number. Keep anything that needs new native capabilities, such as new entitlements or new custom widget kinds, in a store release, and ship everything else through code push.
