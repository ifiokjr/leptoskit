# Code push

An app shell that went through store review is mostly the same every release: the runtime, the native widgets, the permissions. What changes is the interface, and on native platforms the interface is a wasm bundle. Code push ships that bundle over the air, so an interface change reaches devices without another store review.

The iOS and Android apps that `leptoskit new` generates can install code push releases. The generated macOS, Windows, Linux, and HarmonyOS apps can't yet.

To see a release's whole life in one process — signing, serving, staging on a device, a delta, refusing a release that crashes, and mirroring a channel to a bucket — run the `release_lifecycle` example in [`leptoskit_codepush`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_codepush/examples/release_lifecycle.rs): `cargo run -p leptoskit_codepush --example release_lifecycle`.

## The release flow

```bash
leptoskit codepush keygen --out ~/.config/leptoskit/codepush.key
leptoskit native bundle --target ios --out dist/ios
leptoskit codepush release --from dist/ios --platform ios --channel production --signing-key ~/.config/leptoskit/codepush.key
leptoskit codepush publish --platform ios --endpoint https://s3.us-west-004.backblazeb2.com --bucket my-app-updates --region us-west-004
```

1. **`keygen`** creates your publisher key, once. It writes the secret key to a file only you can read and prints the public key for `codepush.public_keys`. The public key ships inside the app shell.
2. **`native bundle`** compiles the selected platform's guest to `dist/ios/app.wasm` and writes `dist/ios/runtime.json`, which names the native ABI, platform-specific shell contract, and plugins the guest needs.
3. **`release`** packs a built directory into a release: a single-file container with a content-hashed manifest, signed with your publisher key. It also computes a binary delta from the channel's previous release, and keeps it when it's at most three quarters of the size of the full container.
4. **`publish`** uploads the channel to an S3-compatible bucket. A CDN in front of the bucket is the whole registry. If you'd rather run a server, `leptoskit codepush serve` serves the same files over HTTP under `/api/codepush`.
5. **Devices download, try, and confirm.** In the generated iOS and Android hosts, the user taps **Check for update**. The [`CodePushClient`](https://docs.rs/leptoskit_codepush) reads the channel's `index.json` and checks its signature before downloading anything. If the release is newer than what the device runs, it asks for the delta from its exact current release, downloads the full container if there isn't one, and verifies every hash. `InstalledBundles` then checks `runtime.json` against the installed shell and stores the signed container. **Activate update** runs it as a trial, and the release becomes the device's working release only after it renders native controls. See [native application projects](../platforms/host-projects.md#code-push-in-the-generated-apps) for the full lifecycle.

## Code push with no server

A release never changes once it's published, and every file in it is addressed by its version or by a hash. The only thing that changes is which release is newest. So the registry is a tree of static files that any object store or CDN can serve:

```text
<app>/<channel>/index.json                                 the newest release: its manifest and signature
<app>/<channel>/<version>+<build>/release.lkb              the container
<app>/<channel>/<version>+<build>/release.sig              the publisher signature
<app>/<channel>/<version>+<build>/deltas/<hash>.jsondelta  the delta from the container with that SHA-256
<app>/<channel>/<version>+<build>/approvals/<hash>.json    source approvals of the release with that bundle hash
```

A device makes plain GET requests and nothing else:

1. `index.json`. A 404 means the channel has no releases yet.
2. `approvals/<bundle hash>.json`, only when the app [requires source approvals](#requiring-source-approvals). A 404 means the release has none yet.
3. `deltas/<sha256 of my container>.jsondelta`. A 404 means there's no delta from what the device runs, so it moves on.
4. `release.lkb`, only when there was no usable delta.

A delta carries only what changed. The container compresses each file in chunks of about 64 KiB, and a chunk whose bytes didn't change compresses to exactly what it did in the previous release, so the delta copies it from the container the device already has. An update that rebuilds one part of your wasm module ships as the few chunks around that part. An update that changes most of the module has no delta worth keeping, and neither does a device still running the bundle that shipped in the store build, since no release was ever computed against it: those devices download the full container.

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

Each platform gets its own update channel. In a project created with `leptoskit new`, a channel named `production` becomes `production-ios` for iOS and `production-android` for Android, and the app, its built-in release, and the publisher all use that name. Keep `codepush.channel` and `--channel` set to the plain name, and pass `--platform` to `release`, `publish`, `pull`, and `check`: each platform then has its own newest release and its own build numbers. A release's `runtime.json` must match the platform you pass, and the CLI checks that before it reads the signing key. Older projects with a single native platform keep their channel names as they are.

### `leptoskit codepush keygen`

| Flag           | Default | Meaning                                                                                                          |
| -------------- | ------- | ---------------------------------------------------------------------------------------------------------------- |
| `--out <PATH>` | —       | Where to write the secret key. An existing file is kept.                                                         |
| `--approver`   | off     | Creates an approver key, and prints its public key for `codepush.provenance.approvers` instead of `public_keys`. |

The secret key is 64 hex characters. Keep it out of version control, and never paste it into logs or issues.

### `leptoskit codepush release`

| Flag                    | Default      | Meaning                                    |
| ----------------------- | ------------ | ------------------------------------------ |
| `--from <DIR>`          | `dist`       | The built directory to release.            |
| `--store <DIR>`         | `releases`   | The release store.                         |
| `--channel <NAME>`      | `production` | The channel to publish to.                 |
| `--platform <PLATFORM>` | —            | The bundle's application platform.         |
| `--mandatory`           | off          | Asks devices to install without prompting. |
| `--signing-key <PATH>`  | —            | The file holding the publisher key.        |

Every release is signed. Without `--signing-key`, the key is read from the `LEPTOSKIT_CODEPUSH_SIGNING_KEY` environment variable, which suits a CI secret. When `codepush.public_keys` is set, releasing with a key that isn't in it is an error, since no app would install the result.

The release is versioned as `app.version+<build>`. The build number goes up by one for each release of the same version. Releasing a version older than the channel's latest is an error, and so is releasing a version and build that already exist. A published release never changes.

### `leptoskit codepush publish`

| Flag                    | Default      | Meaning                                                 |
| ----------------------- | ------------ | ------------------------------------------------------- |
| `--store <DIR>`         | `releases`   | The release store.                                      |
| `--channel <NAME>`      | `production` | The channel to publish.                                 |
| `--platform <PLATFORM>` | —            | The application platform to publish.                    |
| `--endpoint <URL>`      | —            | The S3 API endpoint of the bucket's provider.           |
| `--bucket <NAME>`       | —            | The bucket.                                             |
| `--region <REGION>`     | `auto`       | The bucket's region. `auto` suits R2.                   |
| `--prefix <PATH>`       | —            | A key prefix, so the bucket can hold other things too.  |
| `--path-style`          | off          | Address the bucket as `<endpoint>/<bucket>`, for MinIO. |

Credentials come only from the environment: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and `AWS_SESSION_TOKEN` for temporary credentials. They're never printed. The key only needs to read and write objects in the bucket.

`publish` uploads every release newer than the one the bucket's `index.json` names, oldest first, then points the index at the store's newest release. It refuses to:

- roll the channel back, when the bucket already names a newer release than the store holds;
- replace a file, when the bucket has different contents for a release or file the store also has, because another store released the same version and build;
- publish an unsigned release.

`release` numbers builds and computes deltas from the local store, so the store needs the bucket's newest release before you release. On a machine that keeps `releases/`, that's already true. On a CI runner that starts empty, run `pull` first. Run one publisher per channel at a time.

### `leptoskit codepush pull`

Copies the channel's newest release from the bucket into the store. It takes the same flags and credentials as `publish`.

Before anything is written, the release has to be signed by a key in `codepush.public_keys`, its container has to carry exactly the signed manifest, which is checked before anything in it is decompressed, and every file has to match its hash in that manifest. So a tampered bucket can't plant a release for your next build to number from or patch against. Only the newest release is pulled. That's all `release` needs: the build to count up from, and the container devices run, which the next delta is computed against. Pulling when the store already has the release changes nothing, and an empty bucket leaves the store empty.

A stateless CI job is then three commands:

```bash
leptoskit codepush pull --platform ios --endpoint "$S3_ENDPOINT" --bucket my-app-updates --region us-west-004
leptoskit codepush release --from dist/ios --platform ios --channel production
leptoskit codepush publish --platform ios --endpoint "$S3_ENDPOINT" --bucket my-app-updates --region us-west-004
```

The signing key comes from `LEPTOSKIT_CODEPUSH_SIGNING_KEY`, and the bucket credentials from `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`.

### `leptoskit codepush serve`

| Flag            | Default    | Meaning                                           |
| --------------- | ---------- | ------------------------------------------------- |
| `--store <DIR>` | `releases` | The release store.                                |
| `--dir <DIR>`   | `dist`     | Static files served next to the registry.         |
| `--port <PORT>` | `4100`     | The port. The address comes from `serve.address`. |

`serve` answers with the same files a published bucket holds, so devices can't tell the two apart. It sends `index.json` with `Cache-Control: no-cache`, which means a new release shows up on the next check. The older `check`, `download`, and `delta` endpoints still work too.

Downloads stream in 64 KiB chunks and support Range and HEAD requests. Each router allows 16 active responses and returns 503 when they are all occupied. Put a proxy or CDN in front of a public origin to limit sustained request rates and slow connections. Dynamic checks use hashes saved when releases are ingested or imported; older stores without these hashes offer full downloads until the releases are imported again.

### `leptoskit codepush check`

Asks the store what a device on a given release would be offered:

| Flag                    | Default      | Meaning                            |
| ----------------------- | ------------ | ---------------------------------- |
| `--store <DIR>`         | `releases`   | The release store.                 |
| `--channel <NAME>`      | `production` | The channel to check.              |
| `--platform <PLATFORM>` | —            | The application platform to check. |
| `--version <SEMVER>`    | `0.1.0`      | The version the device runs.       |
| `--build <N>`           | `1`          | The build number the device runs.  |

### `leptoskit codepush approve`

Run by an approver, not the publisher. It unpacks a release, compares it with a directory the approver built themselves, and signs an approval only when both hold exactly the same files with exactly the same bytes. Otherwise it names the first file that differs, is missing, or is extra, and writes nothing.

| Flag                    | Default      | Meaning                                                                                 |
| ----------------------- | ------------ | --------------------------------------------------------------------------------------- |
| `--release <PATH>`      | —            | The release container to approve, such as a `release.lkb` downloaded from the registry. |
| `--version <SEMVER>`    | —            | Instead of `--release`, the version of a release in the store. Needs `--build`.         |
| `--build <N>`           | —            | The build of that release.                                                              |
| `--store <DIR>`         | `releases`   | The release store, with `--version`.                                                    |
| `--channel <NAME>`      | `production` | The channel, with `--version`.                                                          |
| `--platform <PLATFORM>` | —            | The application platform, with `--version`.                                             |
| `--rebuilt <DIR>`       | —            | The directory you built yourself, such as the `--out` of `leptoskit native bundle`.     |
| `--repository <URL>`    | —            | The public GitHub repository you built from, `https://github.com/<owner>/<repo>`.       |
| `--commit <SHA>`        | —            | The full commit id you built, as `git rev-parse HEAD` prints it.                        |
| `--approver-key <PATH>` | —            | The file holding your approver key.                                                     |
| `--out <PATH>`          | —            | Where to write the signed approval.                                                     |

Without `--approver-key`, the key is read from the `LEPTOSKIT_CODEPUSH_APPROVER_KEY` environment variable. Run inside a leptoskit project, `approve` refuses a key that's in the project's `codepush.public_keys`: an approver has to be someone other than the publisher.

### `leptoskit codepush attach`

Attaches approval files to the releases they approve in the store, so `publish` uploads them and `serve` serves them. The releases and their signatures don't change.

| Flag            | Default    | Meaning                                                |
| --------------- | ---------- | ------------------------------------------------------ |
| `--store <DIR>` | `releases` | The release store.                                     |
| `<APPROVAL>...` | —          | The approval files `leptoskit codepush approve` wrote. |

Every approval has to verify and name a release in the store, or nothing is attached. When `codepush.provenance` is set, an approval signed by a key it doesn't list, or for another repository, is refused too, since no app would count it, and `attach` says whether each release now has enough approvals.

## What the client checks

Everything a device downloads is untrusted until it has been verified:

- `index.json` has to carry an ed25519 signature, made by a key in the client's trusted set, over the manifest hash. The manifest names the app, channel, version, build, the mandatory flag, and every file's hash, so the signature covers all of them. This is checked before anything is downloaded.
- The release has to be for the client's own app and channel.
- The release has to be newer than the one the client runs, comparing version and then build. An old release stays validly signed forever, so this is what stops a registry from rolling devices back or replaying a release they already have. An index that isn't newer means "up to date". That's also what a device sees when a CDN still serves the index from before it updated.
- The container's manifest has to be exactly the signed one, which is checked before anything is decompressed, and every file has to match its hash in the manifest.
- Each file has to decompress to exactly its declared size, which stops decompression bombs. A container with anything after its last file, or after a file's compressed data, is rejected.
- Manifest paths have to be relative and stay inside the bundle. Absolute paths and `..` are rejected before anything touches the disk.
- `index.json` is read on every check, so the client stops reading it after 2 MiB.
- A container has to fit the client's size limit, whether it's downloaded whole or rebuilt from a delta. The limit is 32 MiB, the largest container a client can read. A delta's output size is checked against it before any memory is allocated.
- If a delta can't be fetched, doesn't parse, or doesn't apply, the client falls back to the full download.
- `CodePushClient::sync` replaces its staging directory whole, so no file of the previous release survives the update, and a failed sync leaves the previous staging as it was.
- When the app [requires source approvals](#requiring-source-approvals), enough of its approvers have to approve the release, all naming the same commit, before the container is downloaded.

A release that fails these checks is an error, never a silent "up to date", and nothing is staged. In the apps leptoskit generates, a validly signed release that the installed app can't run, because it needs a newer shell, ABI, or plugin, is remembered and isn't downloaded again on later checks. A new store build of the app forgets that list.

The generated iOS and Android apps also refuse a release older than the newest one the device has confirmed, even after losing that release. Say a device confirmed build 5 and later can't read it, because the file was damaged or the disk returned an error. It goes back to running the bundle from the store build, but a stale or hostile index that offers build 4 is still reported as up to date. The device records the newest confirmed release in a small file of its own, apart from the releases it downloaded, and keeps a second copy with its other update state, so either one is enough. If both are damaged, it uses the newest release it can still read. A new store build starts over from the bundle it ships.

Your platform's `Transport` has to report a 404 as `CodePushError::NotFound`. That's how the client tells "no releases yet" and "no delta from here" apart from a network failure.

### What signing doesn't cover

The registry, whether that's a server, a bucket, or a CDN, can still withhold updates: it can keep serving an old `index.json` or serve nothing at all, and the client can't tell that apart from a quiet week. Serve the registry over HTTPS so a network attacker can't do the same.

The client compares offers against the release you tell it is running. Pass the release the shell actually loads, whether that's the bundle from the store build or the last staged update. Otherwise an older signed release can look new.

Anyone who holds the secret key can ship code to your users. Keep it in a secret store, not in the repository. A signature also says nothing about what a release was built from; to have devices check that, [require source approvals](#requiring-source-approvals).

### Rotating keys

`TrustedKeys` holds a set of keys, so a rotation never strands devices:

1. Generate the new key and add its public key to `codepush.public_keys` next to the old one.
2. Ship that shell through the store and wait for it to reach your users.
3. Sign releases with the new key.
4. Remove the old public key in a later store release.

## Requiring source approvals

A publisher signature tells a device who shipped a release. It doesn't tell anyone what the release was built from: whoever holds your publisher key can sign a bundle built from code that was never pushed anywhere. If your app's source is public on GitHub, you can ask independent people, called approvers, to rebuild each release from a commit and vouch that they got the same files. Your app then installs only releases that enough of them vouched for.

The iOS and Android apps `leptoskit new` generates check approvals before they download a release and again every time they launch one. The generated macOS, Windows, and Linux apps don't install code push releases yet, so there is nothing for them to check. HarmonyOS has no code push, and web apps don't use it.

### An example

The maintainers of a notes app ask two people, Alice and Bob, to approve every release, and require both. Each of them creates an approver key once and sends the maintainers the public key it prints:

```bash
leptoskit codepush keygen --approver --out ~/.config/leptoskit/approver.key
```

The app ships with those keys in `leptoskit.toml`:

```toml
[codepush]
enabled = true
server = "https://updates.example.com"
public_keys = ["<your publisher key>"]

[codepush.provenance]
repository = "https://github.com/example/notes"
approvers = ["<Alice's approver key>", "<Bob's approver key>"]
threshold = 2
```

You release from a commit you've pushed, and tell the approvers the version, the build, and the commit:

```bash
git push origin main
leptoskit native bundle --target ios --out dist/ios
leptoskit codepush release --from dist/ios --platform ios --channel production
leptoskit codepush publish --platform ios --endpoint https://s3.us-west-004.backblazeb2.com --bucket notes-updates --region us-west-004
```

Alice checks out that commit, builds it on their own machine, downloads the release from your registry (for iOS, the channel is `production-ios`), and approves it:

```bash
git clone https://github.com/example/notes && cd notes
git checkout 3f2c1b0e9d8a7c6b5a4f3e2d1c0b9a8f7e6d5c4b
leptoskit native bundle --target ios --out rebuilt/ios
curl -o release.lkb "https://updates.example.com/notes/production-ios/1.4.0%2B2/release.lkb"
leptoskit codepush approve --release release.lkb --rebuilt rebuilt/ios --repository https://github.com/example/notes --commit 3f2c1b0e9d8a7c6b5a4f3e2d1c0b9a8f7e6d5c4b --approver-key ~/.config/leptoskit/approver.key --out alice.json
```

Bob does the same. They send you `alice.json` and `bob.json`, and you attach them and publish again:

```bash
leptoskit codepush attach alice.json bob.json
leptoskit codepush publish --platform ios --endpoint https://s3.us-west-004.backblazeb2.com --bucket notes-updates --region us-west-004
```

Until both approvals are published, a device that checks for an update gets an error saying the release has 1 of the 2 source approvals the app requires, and downloads nothing. Once they are, it installs the release as usual.

### What an approval means

An approval says: "I built this commit of this repository myself, and got exactly the files in this release." It names the app, channel, version, build, and the release's bundle hash, which covers every file, and it is signed with the approver's key.

It doesn't say the code is safe, or that anyone reviewed it. It also says nothing about the installed app itself: the host, the trusted keys, and this policy came from the App Store or Google Play, and are trusted because of that.

### What a device checks

A device counts an approval only when:

- it's signed by one of `approvers`. An approval signed by any other key, your publisher key included, counts for nothing. A key can't be in both `public_keys` and `approvers`;
- its signature covers the exact statement it came with, so an edited statement counts for nothing;
- it names this exact release and the `repository` in the policy.

Each approver counts once, however many approvals they signed. Every approval that counts has to name the same commit: if two approvers name different commits, the device refuses the release, whatever the threshold.

The approvals are checked after `index.json` and before anything else of the release is downloaded. The device keeps them next to the release and checks them again every time it loads it. If they no longer verify, because the stored file was changed or deleted, the release doesn't run, and the device falls back to its last working release.

The bundle that shipped in the store build needs no approvals: it's part of the installed app. Files inside a release are only ever bundle files, so a file named `approvals.json` in your `dist/` approves nothing. When a store update adds a policy, releases downloaded before it don't run any more, and devices go back to the store build's bundle until an approved release arrives.

The policy lives in the app the store installed, so a release can't change it: approvers change only with a store release. Without `codepush.provenance`, a publisher signature is enough, as before.

### Rebuilding the same bytes

Approvals only work when building the same commit gives the same bytes on someone else's machine. Pin the Rust toolchain in `rust-toolchain.toml`, commit `Cargo.lock`, and don't put timestamps, absolute paths, or machine names into your bundle. When something differs, `approve` names the first file that does, so the approver can tell you what to fix.

### Approvals in the registry

A release's approvals live at `<app>/<channel>/<version>+<build>/approvals/<bundle hash>.json`. Unlike every other release file, it grows after the release is published, so it is cached for 60 seconds like `index.json`, and `serve` sends it with `Cache-Control: no-cache`. `publish` uploads the store's approvals for every release, keeping any approvals the bucket already has. A bucket or CDN can withhold approvals, but it can't forge one.

## Delivering updates safely

An activated update is a trial. If it traps, the host rejects it, keeps the working release on screen, and refuses that release from then on. A refused release isn't downloaded again: while the channel still names it, a check reads only `index.json` and reports the device up to date, until you publish a newer release. A trial that never renders is never confirmed, so like a trial cut short by the app being killed, it is rejected on the next launch. This only covers startup: a release that renders and then misbehaves stays installed. To roll that back, release the previous build again under a new build number.

Keep anything that needs new native capabilities, such as new plugins, entitlements, or custom widget kinds, in a store release, and change `native.update_contract` in that release. Bundles whose `runtime.json` names a different ABI, contract, or a plugin the shell lacks are refused before they are stored. Interpreted code is a plausible fit for the App Store and Google Play rules on downloaded code, but it is not blanket approval: an update that changes what the app is for can still need review.
