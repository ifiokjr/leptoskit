# Plan: publishing to the stores

Status: **approved** (revision 2), with phase 1 and the skill on `main`. "Progress" at the end records what each pull request delivered; the Apple, Google Play, listing, and CI phases remain.

Revision 2 adds two things:

- Every run of `leptoskit publish` ends with the exact steps a person still has to take in each store's dashboard.
- `leptoskit skill` gives coding agents leptoskit's skill, including how to finish those dashboard steps with computer use.

It also records the answers to the open questions.

Decisions:

- **P1:** `leptoskit publish` is a core command. It takes an app from source to a store's review queue, for the extension stores first and then the app stores.
- **P2:** publishing has two steps on every store: **upload** puts a build where the store holds it as a draft, a test build, or an internal track. **Submit** sends it for review or releases it. `leptoskit publish <store>` uploads; `--submit` also submits. Nothing reaches users without `--submit`.
- **P3:** credentials come from environment variables or a credentials file outside the project, never from `leptoskit.toml`, and are never printed.
- **P4:** `--dry-run` runs every check and builds every package without contacting a store. CI runs it on every example that has a `[publish]` table.
- **P5:** one version, `app.version`, drives every store's version. Build numbers can come from the store itself.
- **P6:** some steps have no API: creating a store listing, privacy forms, permission justifications, content ratings. `leptoskit publish` never leaves them implicit. Every run ends by printing exactly what is left, store by store, with links and the text to paste. The same steps are written to a file and are available as JSON.
- **P7:** leptoskit ships an agent skill. `leptoskit skill` prints it, and `leptoskit skill install` installs it. Both fetch the copy from the GitHub tag that matches the CLI's version. The skill teaches an agent to run a whole release, including driving the store dashboards with computer use, and to stop for the steps only a person may take.

The [extension plan](extension-plan.md) depends on this plan for its phase 6, and the [debugger](devtools-plan.md) publishes its DevTools panel with it.

## The problem

Today leptoskit stops at artifacts:

- **Web:** a build directory.
- **Android:** an APK (`assembleRelease`, `crates/leptoskit_cli/src/native/mod.rs:435`). Release signing is configured by hand in `android/app/build.gradle.kts`, and Google Play wants an App Bundle, which leptoskit does not build.
- **iOS:** `xcodebuild` builds the generated project (`native/mod.rs:443`). There is no archive, no export, and no upload.
- **macOS:** an app bundle signed ad hoc (`native/desktop.rs:117`), which Gatekeeper refuses on any other Mac.
- **HarmonyOS:** signing and AppGallery happen in DevEco Studio.

[Code push](../../docs/src/guide/05-code-push.md) replaces screens after an install, but the first install, and every release that changes native code, still goes through a store. That last step is where most app authors lose a day: certificates, provisioning, keystores, store-specific version rules, source archives, and five different upload APIs. Extension authors face the same thing with three stores and a fourth inside Xcode.

leptoskit knows the app's identity, version, permissions, platforms, and build outputs already. It can do the last step.

## What the app author writes

```toml
[publish.chrome]
publisher_id = "a1b2c3"
item_id = "abcdefghijklmnopabcdefghijklmnop" # created once in the developer dashboard

[publish.edge]
product_id = "00000000-0000-0000-0000-000000000000"

[publish.firefox]
channel = "listed" # or "unlisted" for a signed .xpi you host yourself

[publish.app_store]
app_id = "1234567890" # the app's Apple ID in App Store Connect
team_id = "ABCDE12345"
testflight_groups = ["Internal"]

[publish.play]
track = "internal" # internal, alpha, beta, or production
rollout = 0.1 # with --submit on production: a staged rollout

[publish.macos]
distribution = "developer_id" # notarized, for download outside the Mac App Store
```

```bash
leptoskit publish --dry-run              # every configured store: build, check, upload nothing
leptoskit publish chrome firefox edge    # upload the extension as a draft to three stores
leptoskit publish app_store --submit     # upload to TestFlight, then submit for App Store review
leptoskit publish play --submit          # upload to the configured track and release it
leptoskit publish credentials            # which credentials each configured store needs, and which are missing
```

`leptoskit publish` builds what it uploads, in release mode, from a clean `.leptoskit/publish/` directory, so a stale local build is never uploaded. Before it uploads anything, it asks each store for the latest version, so a version that is not newer fails locally rather than in review.

## Credentials

| Store                  | What it needs                                                                                 | Environment variables                                                                                                                                                          |
| ---------------------- | --------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Chrome Web Store       | A Google Cloud service account added to the publisher in the developer dashboard              | `LEPTOSKIT_PUBLISH_CHROME_SERVICE_ACCOUNT` (a path to the JSON key, or the JSON itself)                                                                                        |
| Microsoft Edge Add-ons | A Publish API client ID and API key from Partner Center. Keys expire after 72 days.           | `LEPTOSKIT_PUBLISH_EDGE_CLIENT_ID`, `LEPTOSKIT_PUBLISH_EDGE_API_KEY`                                                                                                           |
| addons.mozilla.org     | An API key and secret, used to sign a short-lived JWT for each request                        | `LEPTOSKIT_PUBLISH_FIREFOX_JWT_ISSUER`, `LEPTOSKIT_PUBLISH_FIREFOX_JWT_SECRET`                                                                                                 |
| App Store Connect      | An App Store Connect API key (issuer ID, key ID, `.p8`)                                       | `LEPTOSKIT_PUBLISH_APPLE_ISSUER_ID`, `LEPTOSKIT_PUBLISH_APPLE_KEY_ID`, `LEPTOSKIT_PUBLISH_APPLE_KEY`                                                                           |
| Developer ID (macOS)   | A Developer ID Application certificate in the keychain, and the same API key for notarization | `LEPTOSKIT_PUBLISH_MACOS_IDENTITY`, plus the App Store Connect variables                                                                                                       |
| Google Play            | A service account with access to the app in Play Console, and the upload keystore             | `LEPTOSKIT_PUBLISH_PLAY_SERVICE_ACCOUNT`, `LEPTOSKIT_ANDROID_KEYSTORE`, `LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD`, `LEPTOSKIT_ANDROID_KEY_ALIAS`, `LEPTOSKIT_ANDROID_KEY_PASSWORD` |

- `--credentials <file>` reads the same names from a TOML file. The command refuses a file inside the project directory unless git ignores it.
- `leptoskit publish credentials` lists what each configured store needs, which variables are set, and where to create the missing ones. It shows names, never values.
- Values are held in a `Secret` type that redacts itself in `Debug` and in error messages, like the auth pack's `secret` fields.
- Apple signing uses the API key with `-allowProvisioningUpdates`, so CI needs no certificates or provisioning profiles installed by hand. The Developer ID certificate is the exception, because notarized apps must be signed with it.
- Android release signing moves out of the hand-edited Gradle file: the generated `build.gradle.kts` reads the four `LEPTOSKIT_ANDROID_*` variables when they are set, and leaves the release unsigned when they are not.

## Versions and build numbers

- Every store gets its version from `app.version` (`crates/leptoskit_config/src/lib.rs:115`).
- Extension stores accept only one to four dot-separated integers, so `1.4.0-beta.2` becomes `1.4.0`, and Chromium's `version_name` keeps the full string. Two uploads that map to the same number are refused before upload.
- Apple's `CFBundleVersion` and Android's `versionCode` must increase on every upload. `--build-number auto` asks the store for its highest build number and adds one. `--build-number <n>` sets it. The default is `auto`, and `--dry-run` reports the number it would use.

## Every store

### Chrome Web Store

- **API:** Chrome Web Store API v2 ([announcement](https://developer.chrome.com/blog/cws-api-v2)). It authenticates with a service account; v1 was retired on 15 October 2026.
- **Upload** sends `chromium.zip` to the item. **Submit** calls `:publish`. `--staged` uses `STAGED_PUBLISH`, so an approved version waits for a later `leptoskit publish chrome --release-staged`.
- **Once, by hand:** the item is created in the developer dashboard, together with its listing, privacy practices, and a justification for each permission. The API cannot set them. `leptoskit publish` prints a checklist of the permissions in the manifest, so the justifications can be written once from `store/permissions.toml` and pasted in.

### Microsoft Edge Add-ons

- **API:** the Edge Add-ons Publish API, with an API key and client ID ([announcement](https://blogs.windows.com/msedgedev/2024/09/30/enhanced-security-for-extensions-with-new-publish-api/)).
- **Upload** sends `chromium.zip` as the product's draft package and polls until it is processed. **Submit** publishes the draft with the release notes from `store/<locale>/release_notes.md`.
- **Key expiry:** API keys expire after 72 days. An expired key gets an error that names the Partner Center page where a new one is made.

### addons.mozilla.org

- **API:** AMO API v5, add-on submission. It authenticates with a JWT signed from the API key and secret.
- **Upload** sends `firefox.zip` together with `firefox-source.zip`. AMO asks for the source of machine-generated code, and wasm built from Rust counts. The extension build already produces the archive and its `BUILD.md` (see the extension plan).
- **Channels:** `listed` goes into AMO's review queue, so **submit** has nothing more to do; `leptoskit publish firefox` without `--submit` stops before creating the version and says so. `unlisted` returns a signed `.xpi` for self-distribution, which is written to `.leptoskit/publish/firefox/`.
- **Listing:** name, summary, and description can be set through the API, so they come from `store/`.

### App Store Connect: iOS, and Safari on iOS and macOS

- **Archive:** `xcodebuild archive` on the generated project, with automatic signing through the API key.
- **Upload:** `xcodebuild -exportArchive` with an `ExportOptions.plist` whose `method` is `app-store-connect` and whose `destination` is `upload`, so Xcode uploads the build itself. The Safari extension from the extension plan is embedded in the app, so it ships with it.
- **TestFlight:** the command waits for the build to process, then adds it to `testflight_groups` through the App Store Connect API.
- **Submit:** creates or updates the App Store version, attaches the build, sets "What's New" from `store/<locale>/release_notes.md`, and creates a review submission.
- **Listing:** the App Store Connect API covers descriptions, keywords, and screenshots, so they come from `store/`.
- **Safari without a native app:** an app whose only platform is `extension` has no Xcode project. App Store Connect's Safari Web Extension Packager can build the app from the extension's files ([Apple](https://developer.apple.com/documentation/safariservices/packaging-and-distributing-safari-web-extensions-with-app-store-connect)). Developers report that its upload section is missing for some accounts, so this plan generates a minimal containing app instead, and uses the packager once it is reliable.

### macOS outside the Mac App Store

- **Signing:** the assembled bundle and its Safari extension are signed with the Developer ID identity, with the hardened runtime and a secure timestamp. This replaces the ad hoc signature.
- **Notarization:** `xcrun notarytool submit --wait` with the API key, then `xcrun stapler staple`.
- **Output:** a notarized `.dmg` and `.zip` in `.leptoskit/publish/macos/`. "Upload" here means producing them. With `--submit`, they are attached to a GitHub release when `[publish.macos].github_release` is set.
- **The Mac App Store** needs the app sandboxed, which the macOS host is not today. That is a separate decision, listed under later work.

### Google Play

- **Bundle:** `bundleRelease` produces an App Bundle beside the APK. Play App Signing holds the app signing key, and leptoskit signs the bundle with the upload key.
- **API:** Google Play Developer API v3 edits: insert an edit, upload the bundle, set the track, and commit.
- **Upload** commits the bundle to the configured track as a draft release. **Submit** completes the release, with `rollout` as the user fraction on production.
- **Once, by hand:** the app is created in Play Console, together with its content rating and data safety form. The API cannot create an app.

## Store listings

Text and images that the stores' APIs accept live in the project, so they are reviewed like code:

```text
store/
├── listing.toml              privacy policy URL, support URL, categories, keywords
├── permissions.toml          one sentence per permission, for reviewers
├── en-US/
│   ├── description.md
│   └── release_notes.md
└── screenshots/
    ├── iphone-6.9/  ipad-13/  android-phone/  chrome-1280x800/
    └── screenshots.toml      which routes to capture, on which devices
```

- `leptoskit publish` uploads what each store's API accepts, and prints what must still be set by hand.
- **Screenshots from the app.** `leptoskit publish screenshots` opens each route in `screenshots.toml` on the device sizes the stores require and saves the screenshots. It uses the device control that `leptoskit device` and `leptoskit_testing` already provide. Re-running it after a design change refreshes every store's screenshots.

## What to do next

Every `leptoskit publish` run, including `--dry-run`, ends with a "What to do next" section. It lists every step that is still left, for every configured store, in the order to do them. Each step has:

- **what** to do, in one sentence;
- **where**: the exact dashboard URL, built from the IDs in `[publish]`, such as the item's edit page in the Chrome Web Store developer dashboard;
- **what to enter**: generated from the project, ready to paste, such as the justification for each permission from `store/permissions.toml`, the privacy policy URL, or the release notes;
- **why** the API cannot do it;
- **done when**: how leptoskit will know. A later run checks through the store's API where it can, and drops the step once it is done.

For example, after the first Chrome upload of Commonplace:

```text
What to do next

Chrome Web Store: Commonplace 1.0.0 is uploaded as a draft. Three steps are left before you can submit it.

  1. Fill in the privacy practices.
     Open  https://chrome.google.com/webstore/devconsole/a1b2c3/abcdefghijklmnopabcdefghijklmnop/edit/privacy
     Paste each justification below into the field with the same name:
       activeTab     Reads the page you clip from, only when you press the toolbar button.
       scripting     Shows the "Clip" button over the text you select.
       …
     Single purpose: Save quotes from the web into your Commonplace research projects.
     Why by hand: the Chrome Web Store API has no privacy practices endpoint.

  2. Add at least one 1280x800 screenshot.
     Upload  store/screenshots/chrome-1280x800/*.png  (3 files, made by `leptoskit publish screenshots`)
     …

  3. Run `leptoskit publish chrome --submit`.

App Store: build 42 is in TestFlight. Nothing is left until you submit.

Wrote these steps to .leptoskit/publish/next-steps.md (and next-steps.json).
```

- **Kinds of step.** Each step is marked `agent` or `person`. An agent can do an `agent` step in a dashboard. A `person` step needs the account holder:
  - signing in, and two-factor codes;
  - accepting agreements, and anything involving payment or tax;
  - legal declarations, such as export compliance and a content rating's answers;
  - the final decision to release.
- **Formats.** `--json` prints the steps as JSON instead of text, for agents and CI.
- **Nothing to do.** When nothing is left, the section says so in one line.

## The leptoskit skill

Coding agents learn tools from skills: a `SKILL.md` with instructions, plus reference files the agent reads when it needs them. leptoskit ships one.

```bash
leptoskit skill                          # print the skill to the terminal
leptoskit skill install                  # install into ./.claude/skills/leptoskit and ./.agents/skills/leptoskit
leptoskit skill install --dir <path>     # install somewhere else
leptoskit skill install --global         # install into ~/.claude/skills/leptoskit
```

- **Where it lives.** The skill is `skills/leptoskit/` in this repository. `SKILL.md` covers leptoskit in general and points to one reference file per task, such as `references/publishing.md`, `references/extensions.md`, and `references/devtools.md`.
- **Which version.** `leptoskit skill` downloads the skill from the GitHub tag for the CLI's own version, `v<version>`. A CLI and its skill therefore always describe the same commands. The list of files is `skills/leptoskit/files.txt`, so the CLI fetches exactly those files. A CLI built from a git checkout reads `skills/leptoskit/` from that checkout, or `--ref <branch>` names another revision.
- **Installing.** `install` overwrites only files it wrote before, which it tracks with a `.leptoskit-skill` marker that records the version. It refuses to replace a directory without that marker unless `--force` is given.
- **Printing.** Without `install`, the command prints `SKILL.md` followed by every reference file with a heading, so the output can be pasted into any agent.
- **Offline.** The command fails with the URL it tried, and the command that retries.
- **`leptoskit new`** prints a line that suggests `leptoskit skill install`, rather than writing agent files into every project unasked.

### Releasing with an agent

`references/publishing.md` walks an agent through a whole release:

1. Run `leptoskit publish --dry-run`, and fix each check that fails.
2. Run `leptoskit publish credentials`. If any are missing, ask the person to create them, using the steps the command printed. An agent never creates API keys itself, because that needs the account holder.
3. Run `leptoskit publish <stores>`. Then read `.leptoskit/publish/next-steps.json`.
4. For each `agent` step, use **computer use** (or a browser automation tool, when the agent has one) to open the step's URL and do it:
   - take a screenshot before acting, and after each change;
   - paste the given text exactly as written;
   - upload the listed files.

   Stop and hand over to the person for every `person` step, and whenever the page asks for anything the step did not describe.
5. Run `leptoskit publish` again. The steps it has confirmed disappear. Repeat until only the submission is left.
6. Ask the person before `--submit`. Submitting sends the app to review or to users, and the person decides that.

The reference also lists, for each store, the dashboard pages in order, what each field means, and the reasons stores most often reject an app.

## Checks

`--dry-run` runs these, and so does every upload before it contacts a store:

- the package builds, and its manifest or `Info.plist` is valid;
- the version is newer than the store's latest, when credentials are present;
- every required icon size exists, generated from one source image where the platform allows it;
- every permission in the extension manifest has a sentence in `store/permissions.toml`, and every iOS usage description is set (`validate_ios_usage_descriptions`, `crates/leptoskit_cli/src/native/project.rs:28`);
- the Firefox source archive rebuilds to the same package, in CI;
- the privacy policy URL is set for every store that requires one;
- Android's target SDK meets Play's current minimum.

## Every file this touches

| Area   | Files                                                                                                                                                                                                                     |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Config | `crates/leptoskit_config/src/lib.rs`: `PublishConfig` with one optional table per store, and validation. `schema.rs` and the schema through `schema:update`.                                                              |
| CLI    | `crates/leptoskit_cli/src/cli.rs`: the `Publish` command. New `crates/leptoskit_cli/src/publish/{mod,credentials,checks,version,listing,screenshots}.rs`, and one module per store.                                       |
| Stores | `publish/{chrome,edge,firefox,app_store,play,macos}.rs`, each a small client over `ureq`, which the CLI already uses for code push uploads (`crates/leptoskit_cli/src/s3.rs`).                                            |
| Native | `native/mod.rs:435`: `bundleRelease`. `native/templates/build.gradle.kts`: signing from the environment. `native/project/ios.rs`: archive and export options. `native/desktop.rs`: Developer ID signing and notarization. |
| Docs   | A public guide chapter, "Publishing to the stores", with one section per store. The platform pages link to it instead of saying "configure signing by hand".                                                              |

## Tests

- **Store clients.** Each client runs against a local fake server that records requests, with fixtures taken from each store's documentation: upload, processing, publish, errors, expired keys, and a version that is not newer. No test contacts a real store.
- **Dry runs.** Snapshots of `leptoskit publish --dry-run` output for Commonplace, which covers every store at once.
- **Credentials.** No secret value appears in any output, error, or `Debug` string, checked by a test that sets known values and searches every output for them.
- **Versions.** Mapping semantic versions to each store's format, including the refusals.
- **Real stores.** leptoskit's own DevTools panel extension is published with `leptoskit publish` from the release workflow, so every leptoskit release exercises the Chrome Web Store, Edge, and AMO clients for real.

## Phases and pull requests

**Phase 1: the command and the extension stores.**

1. `feat(config)`: `[publish]`, validation, and schema.
2. `feat(cli)`: `leptoskit publish` with `--dry-run`, credentials, version mapping, checks, and the "What to do next" steps in text and JSON.
3. `feat(cli)`: Chrome Web Store and Edge Add-ons.
4. `feat(cli)`: addons.mozilla.org, with the source archive.

**Phase 2: Apple.**

1. `feat(cli)`: iOS archive, export, upload, and TestFlight, with the Safari extension inside.
2. `feat(cli)`: App Store submission.
3. `feat(cli)`: macOS Developer ID signing, notarization, and stapling.

**Phase 3: Google Play.**

1. `feat(native)`: App Bundles, and release signing from the environment.
2. `feat(cli)`: Play uploads, tracks, and staged rollouts.

**Phase 4: listings and screenshots.**

1. `feat(cli)`: `store/` and listing uploads where the APIs allow.
2. `feat(cli)`: `leptoskit publish screenshots`.

**Phase 5: CI.**

1. `feat(cli)`: `leptoskit new` writes a GitHub Actions workflow that runs `leptoskit publish` on a tag, with the credentials as repository secrets.

**Phase 6: the skill.**

1. `feat(cli)`: `leptoskit skill`, `leptoskit skill install`, and `skills/leptoskit/` with `SKILL.md`.
2. `docs(skill)`: `references/publishing.md`, with the computer-use release flow, then the extension and devtools references as those features land.

The skill's first pull request can land early, because it does not depend on publishing; each later feature adds its reference in its own pull request.

## Later

- The Mac App Store, which needs a sandboxed macOS host.
- The Microsoft Store (MSIX through the Partner Center submission API), Flathub, and Snapcraft.
- AppGallery Connect for HarmonyOS, once that host leaves its experimental status.
- App Store Connect's Safari Web Extension Packager, for extension-only apps, once it works for every account.

## Decided

- **Q1, confirming `--submit`:** in an interactive terminal it asks for confirmation as well, and `--yes` skips the question for scripts. CI is detected and never prompts.
- **Q2, icons:** leptoskit generates every platform's icon sizes from one source image, `app.icon`, because missing sizes are the most common store rejection.

## Progress

- **Phase 1:** `[publish]` in `leptoskit.toml`, validation and the schema, `leptoskit publish` with `--dry-run`, `--submit`, credentials from the environment or a file outside the project, version mapping, the store checks, and the "next steps" report in text and JSON. Chrome Web Store, Edge Add-ons, and addons.mozilla.org upload, wait for their checks, and submit, with the Firefox source archive (#302).
- **Phase 6:** `leptoskit skill` and `leptoskit skill install` write the repository's skill, including the release flow (#302). The reference files grow with each feature: extensions and the debugger are there, Apple and Google Play follow their phases.
- **Phase 2.1–2.2, App Store Connect:** `[publish.app_store]` (`app_id`, `team_id` or `native.apple_team_id`, `testflight_groups`) and the App Store Connect API key (ES256 tokens, 20 minutes). An upload numbers the build from the store's highest (`--build-number`), runs `native prepare ios --release`, archives and exports with `xcodebuild` through a `publish::tools::Runner` seam (`-allowProvisioningUpdates` with the key, `ExportOptions.plist` with `destination = upload`), waits for processing, and adds the build to TestFlight groups, asking for beta review when one is external. `--submit` makes the App Store version editable, sets "What's New", attaches the build, and submits a review submission. The generated Xcode project now reads `CFBundleShortVersionString` and `CFBundleVersion` from `MARKETING_VERSION` and `CURRENT_PROJECT_VERSION`, through a copy of the app's `Info.plist` in `.leptoskit/ios/`, for the app and the Safari extension. `--dry-run` checks and writes the export options, but doesn't archive. The xcodebuild steps are tested by their argument lists; this repository's machines haven't run a real upload.
- **Still open:** the macOS Developer ID and notarization path, Google Play, listing and screenshot uploads, and the generated release workflow (phases 2.3 to 5).
