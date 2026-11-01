# Publishing to the stores

Getting an app into a store is where a release usually loses a day. Every store has its own upload API, its own version rules, its own review form, and pages that can only be filled in by hand. `leptoskit publish` does the uploading, and finishes by telling you exactly what is left in each store's dashboard, with the text to paste.

Today it publishes [browser extensions](../platforms/extension.md) to the Chrome Web Store, Microsoft Edge Add-ons, and addons.mozilla.org, [iOS apps](../platforms/ios.md) to TestFlight and the App Store, [Android apps](../platforms/android.md) to Google Play, and [macOS apps](../platforms/macos.md) as notarized downloads.

## An example

Commonplace's extension goes to all three stores. Name them in `leptoskit.toml`, with the IDs from each store's dashboard. Credentials never go in this file:

```toml
[publish.chrome]
publisher_id = "a1b2c3"
item_id = "abcdefghijklmnopabcdefghijklmnop"

[publish.edge]
product_id = "1f9c…"

[publish.firefox]
channel = "listed" # or "unlisted", for a signed add-on you host yourself
```

Then:

```bash
leptoskit publish --dry-run       # build, check, and list what's left; contact no store
leptoskit publish credentials     # which credentials each store needs, and which are missing
leptoskit publish                 # upload to every configured store, as a draft
leptoskit publish chrome --submit # send it to review
```

## Upload, then submit

Every store has two steps:

- **Upload.** `leptoskit publish` puts the build where the store keeps it as a draft:
  - in the Chrome Web Store, the item's draft;
  - in Edge Add-ons, the product's draft submission;
  - on addons.mozilla.org, a validated upload;
  - in App Store Connect, a build in TestFlight, for the testers in the groups you name;
  - in Google Play, a draft release on the track you name;
  - for macOS, a notarized `.dmg` and `.zip` in `.leptoskit/publish/macos/`.

  Nobody else sees it yet.
- **Submit.** `--submit` sends it for review, and then to users. In a terminal, it asks once more first; `--yes` skips the question for scripts, and CI never asks. `--staged` keeps an approved Chrome Web Store version until you publish it by hand.

Each run builds what it uploads in release mode, so a stale build is never uploaded.

## Credentials

| Store                  | Environment variables                                                                                                                                                                                                                             | Where to get them                                                                                                                                                                       |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Chrome Web Store       | `LEPTOSKIT_PUBLISH_CHROME_SERVICE_ACCOUNT`: the JSON key, or its path                                                                                                                                                                             | A Google Cloud service account, added in the developer dashboard under **Account**, **API access**                                                                                      |
| Microsoft Edge Add-ons | `LEPTOSKIT_PUBLISH_EDGE_CLIENT_ID` and `LEPTOSKIT_PUBLISH_EDGE_API_KEY`                                                                                                                                                                           | Partner Center's **Publish API** page. Keys expire after 72 days.                                                                                                                       |
| addons.mozilla.org     | `LEPTOSKIT_PUBLISH_FIREFOX_JWT_ISSUER` and `LEPTOSKIT_PUBLISH_FIREFOX_JWT_SECRET`                                                                                                                                                                 | The add-on developer hub's **API keys** page                                                                                                                                            |
| App Store Connect      | `LEPTOSKIT_PUBLISH_APPLE_ISSUER_ID`, `LEPTOSKIT_PUBLISH_APPLE_KEY_ID`, and `LEPTOSKIT_PUBLISH_APPLE_KEY`: the `.p8` file's path, or its contents                                                                                                  | **Users and Access**, **Integrations**, **App Store Connect API**: a team key with the App Manager role                                                                                 |
| Developer ID (macOS)   | `LEPTOSKIT_PUBLISH_MACOS_IDENTITY`: the certificate's name, plus the three App Store Connect variables, which notarize the app                                                                                                                    | A **Developer ID Application** certificate, made in Xcode under **Settings**, **Accounts**, **Manage Certificates**; `security find-identity -v -p codesigning` shows its name          |
| Google Play            | `LEPTOSKIT_PUBLISH_PLAY_SERVICE_ACCOUNT`: the JSON key, or its path; and the upload key: `LEPTOSKIT_ANDROID_KEYSTORE` (its absolute path), `LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD`, `LEPTOSKIT_ANDROID_KEY_ALIAS`, `LEPTOSKIT_ANDROID_KEY_PASSWORD` | A Google Cloud service account with the Google Play Android Developer API turned on, invited in Play Console under **Users and permissions**; the upload key from `keytool -genkeypair` |

Set them in the environment, or put them in a TOML file outside the project and pass `--credentials <file>`. A file inside the project is refused unless git ignores it. Values are never printed, in output or in errors. (Xcode prints its own command line, which shows the App Store key's ID and issuer ID, but never the key.)

## The App Store

Your iOS app goes to TestFlight on every upload, and to App Review with `--submit`. If the app has a Safari extension, it is inside the app and goes with it. Create the app once in App Store Connect, then name it:

```toml
[publish.app_store]
app_id = "1234567890" # the Apple ID under App Information
team_id = "ABCDE12345" # or set native.apple_team_id
testflight_groups = ["Internal"]
```

```bash
leptoskit publish app_store          # archive, upload, and add the build to TestFlight
leptoskit publish app_store --submit # also send version app.version to App Review
```

This runs on a Mac with Xcode. An upload:

1. asks App Store Connect for the app's highest build number, and uses the next one. `--build-number 42` picks it instead.
2. builds the app in release mode, then archives it with `xcodebuild`, signed automatically with the API key. You don't install certificates or provisioning profiles; Xcode makes them on first use.
3. uploads the archive, and waits for App Store Connect to process it, which takes from a few minutes to an hour.
4. adds the build to each group in `testflight_groups`. External groups also need Apple's beta review, which the command asks for.

The version is `app.version` (a pre-release label like `-beta.1` is dropped, since Apple takes only numbers), and the build number is the one above. Both are set for the Safari extension too, which Apple requires.

With `--submit`, the command also creates the App Store version (or renames the one you were preparing), sets "What's New" from `store/en-US/release_notes.md`, attaches the build, and submits it for review. Apple refuses the submission until the steps below are done, and says what is missing.

These stay in App Store Connect's own pages, and the steps after each run list them:

- the version's page: description, keywords, support address, and screenshots;
- App Information: the category and the privacy policy's address;
- App Privacy, the age rating, and export compliance, which are legal declarations for the account holder. Set `ITSAppUsesNonExemptEncryption` in your app's `Info.plist` to answer export compliance in every build;
- App Review's contact details, and a demo account if the app needs signing in.

`--dry-run` checks the configuration and writes `.leptoskit/publish/ios/ExportOptions.plist`, but doesn't build the app, since an archive needs your signing team.

## Google Play

Your Android app goes to Google Play as an App Bundle, on the track you name:

```toml
[publish.play]
track = "internal" # internal, alpha, beta, production, or a closed testing track
rollout = 0.1 # with --submit on production: a tenth of users first
```

```bash
leptoskit publish play          # build, sign, and upload a draft release to the track
leptoskit publish play --submit # release it, or start the staged rollout
```

An upload:

1. asks Google Play for the highest version code on any track, and uses the next one. `--build-number 42` picks it instead.
2. builds the App Bundle with Gradle, signed with your upload key. Google Play keeps the key that signs what people install (Play App Signing), so the upload key never reaches users.
3. uploads it, and puts it on the track as a draft. With `--submit`, the release goes to everyone on the track, or, on production with `rollout`, to that fraction of users first.

The Gradle script reads the upload key from the four `LEPTOSKIT_ANDROID_*` variables, so the keystore and its passwords stay out of the project; without them, `leptoskit native build android --release` still builds, unsigned. Release builds now make the App Bundle (`app-release.aab`) next to the APK. The app's `versionName` is `app.version`.

Before it uploads, the command checks the Gradle script: it must read the version code leptoskit gives it, sign with the upload key, and target API 36, the lowest Google Play takes for new apps and updates. Projects made by an earlier `leptoskit new` get the lines to add in the error.

Google Play can't create an app through its API, and takes API uploads only after the first bundle was uploaded in Play Console. The steps after the first run say so, with the bundle's path. These also stay in Play Console:

- the store listing: descriptions, icon, feature graphic, and screenshots;
- App content: the privacy policy, and the data safety, content rating, target audience, and ads declarations, which are the account holder's.

## A notarized macOS download

A Mac opens an app downloaded from the web only if it is signed with a Developer ID and notarized by Apple. `leptoskit publish macos` does both, for download from your own site or a GitHub release:

```toml
[publish.macos]
distribution = "developer_id"
github_release = "you/notes" # optional: attach the files to a release with --submit
```

```bash
leptoskit publish macos          # build, sign, notarize, and staple
leptoskit publish macos --submit # also attach them to the release v<app.version>
```

This runs on a Mac with Xcode and the Developer ID certificate in its keychain. It builds the app in release mode, and then, on a copy in `.leptoskit/publish/macos/`:

1. sets its version to `app.version` (and its build number, with `--build-number <n>`);
2. signs the Safari extension inside it, then the app, with the hardened runtime and a secure timestamp;
3. makes a `.dmg`, signs it, and sends it to Apple's notary service, which takes a few minutes. If Apple refuses it, the error gives the `xcrun notarytool log` command that shows why;
4. staples Apple's ticket to the `.dmg` and the app, so they open without a network connection, and zips the app.

With `github_release` and `--submit`, the `.dmg` and `.zip` are attached to the GitHub release `v<app.version>`, which is made if it's missing; this uses the `gh` command, signed in. Without `github_release`, put the `.dmg` wherever people download your app; the steps after each run remind you.

The Mac App Store isn't supported: it needs the app sandboxed, which the macOS host isn't yet.

## What to do next

Some steps have no API: the Chrome Web Store's listing and privacy practices, Edge's store listing, the first version of a new Firefox add-on, and the App Store's privacy and age rating answers. Every run, including `--dry-run`, ends with them, store by store:

```text
chrome: Commonplace 1.4.0 is uploaded as a draft.

  2. Fill in the privacy practices: the single purpose, a justification for each permission, and the privacy policy's address.
     Open   https://chrome.google.com/webstore/devconsole/a1b2c3/abcdefghijklmnopabcdefghijklmnop/edit/privacy
     activeTab: Reads the page you're on, only when you press the toolbar button or the shortcut, so you can clip from it.
     …
```

Each step says what to do, where, what to enter, why the API can't do it, and how you'll know it's done. Steps marked **(you, the account holder)** are the account holder's own: signing in, accepting agreements, legal declarations, choosing a license, and deciding to release. The same steps are written to `.leptoskit/publish/next-steps.md`, and as JSON to `next-steps.json`, which `--json` also prints.

## Keep the listing with the code

A store's listing text lives in a `store/` folder in your project, so it's reviewed like code. Each upload sends what the store's API takes, and the steps paste the rest:

```text
store/
├── listing.toml          name, keywords, privacy_policy, homepage, support, category
├── permissions.toml      one sentence per permission, for reviewers
├── en-US/
│   ├── summary.md        a sentence or two
│   ├── description.md
│   └── release_notes.md
└── screenshots/
    └── screenshots.toml  the screens `leptoskit publish screenshots` captures
```

```toml
# store/listing.toml
privacy_policy = "https://notes.example.com/privacy"
support = "https://notes.example.com/help"
category = "Productivity"
keywords = ["notes", "research", "quotes"]
```

What goes up with each upload:

| Store                  | Uploaded from `store/`                                                                                           | Left for the steps                          |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| App Store              | description, keywords, summary (as promotional text), support and homepage addresses, privacy policy, What's New | screenshots, category                       |
| Google Play            | name, summary (as the short description), description, release notes                                             | icon, feature graphic, screenshots          |
| addons.mozilla.org     | name, summary, description                                                                                       | screenshots, privacy policy                 |
| Chrome Web Store, Edge | release notes, on Edge                                                                                           | the whole listing: their APIs don't edit it |

Text a store would refuse as too long fails before anything is built: Google Play takes an 80-character summary and a 30-character name, the App Store 100 bytes of keywords and a 170-character summary, and addons.mozilla.org a 250-character summary.

```toml
# store/permissions.toml
activeTab = "Reads the page you're on, only when you press the toolbar button, so you can clip from it."
storage = "Keeps your sign-in and your chosen project on this device."
```

A permission without a sentence shows up in the steps as one to write. Store reviewers reject permissions they can't see a reason for more often than anything else.

## One icon for every platform

A missing icon size is the most common reason a store turns an app away, and every platform wants different ones. Point `app.icon` at one square image, and every build makes the rest:

```toml
[app]
icon = "assets/icon.svg" # or a PNG, 1024×1024 or larger
```

| Platform  | What the build makes                                                                                                                           |
| --------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| iOS       | The app's asset catalog with a 1024×1024 icon, laid over white, since the App Store refuses transparency; Xcode makes the device sizes from it |
| Android   | Launcher icons for every screen density, and an adaptive icon: your image inside the mask's safe zone, on its corner color                     |
| macOS     | `AppIcon.icns`, from 16 to 1024 pixels, inside the app                                                                                         |
| Extension | The 16, 32, 48, and 128 pixel icons, in the manifest                                                                                           |

An SVG is drawn at 1024 pixels; its text isn't, so turn text into shapes first. Design the image to work as a square: iOS rounds its corners, and Android crops it to a circle or another shape. Without `app.icon`, the steps after `leptoskit publish` ask for one.

Projects made before `app.icon` add one line to `android/app/build.gradle.kts`, in the main source set, so the launcher icons reach the app; the Android build warns until it's there:

```kotlin
res.directories += "../../.leptoskit/android/res"
```

## Screenshots from the app

Every store wants screenshots at its own sizes, and they go stale with every design change. `leptoskit publish screenshots` captures them from the app, into `store/screenshots/<size>/`, replacing the ones it made before. Name the sizes and the screens in `store/screenshots/screenshots.toml`:

```toml
url = "http://127.0.0.1:3000" # the app, as `leptoskit dev` serves it

[devices.chrome-1280x800] # the Chrome Web Store and Edge
viewport = [1280, 800]

[devices."iphone-6.9"] # the App Store's largest iPhone size
simulator = "iPhone 17 Pro Max"

[devices.android-phone] # Google Play
android = "emulator-5554"

[[screenshot]]
name = "library"
route = "/library"
# On the phones, the app starts at its first screen; these reach this one.
steps = [{ action = "tap", locator = { by = "text", value = "Library" } }]
wait_for = { by = "text", value = "Your library" }
```

```bash
leptoskit dev                           # in another terminal, for the browser sizes
leptoskit publish screenshots --dry-run # list the files it would make
leptoskit publish screenshots
```

- **A browser size** (`viewport`, and `scale` for a high-density screen) opens each screen's `route` in headless Chrome at exactly that size. It needs Chrome and chromedriver.
- **A simulator or Android size** launches your installed app, takes the screen's `steps`, and captures the app. The steps are the same ones [`leptoskit device`](../platforms/readme.md) and control documents use, so they need Appium with the platform's driver, a booted simulator or running emulator, and the app installed on it.
- A screen's `devices` list limits it to some sizes. Files are numbered in the order the screens are listed: `01-library.png`, `02-…`.

The steps after each run say which folder each store's screenshots come from. The stores' APIs take images, but leptoskit doesn't upload them yet.

## Let an agent finish the release

`leptoskit skill install` gives coding agents, such as Claude Code, leptoskit's skill for the version of the CLI you have. Its publishing guide walks an agent through a release: run the dry run and fix what fails, upload, then work through the steps in each dashboard with computer use. The agent stops for every step marked for the account holder, and never submits without being told to.

```bash
leptoskit skill install          # into .claude/skills and .agents/skills in this project
leptoskit skill install --global # for every project
leptoskit skill                  # print it, to paste into any agent
```

## What doesn't work yet

- **Screenshots and images** aren't uploaded yet: the steps name the folders to upload them from.
- **The Mac App Store** isn't supported. Safari's extension ships inside your iOS app, so it goes through the App Store with the app.
- **Listing text and screenshots** aren't uploaded; they appear in the steps instead.
- **A brand new Firefox add-on** needs its first version submitted on the site. After that, `leptoskit publish firefox --submit` adds versions.
