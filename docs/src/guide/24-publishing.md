# Publishing to the stores

Getting an app into a store is where a release usually loses a day. Every store has its own upload API, its own version rules, its own review form, and pages that can only be filled in by hand. `leptoskit publish` does the uploading, and finishes by telling you exactly what is left in each store's dashboard, with the text to paste.

Today it publishes [browser extensions](../platforms/extension.md) to the Chrome Web Store, Microsoft Edge Add-ons, and addons.mozilla.org, and [iOS apps](../platforms/ios.md) to TestFlight and the App Store.

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
  - in App Store Connect, a build in TestFlight, for the testers in the groups you name.

  Nobody else sees it yet.
- **Submit.** `--submit` sends it for review, and then to users. In a terminal, it asks once more first; `--yes` skips the question for scripts, and CI never asks. `--staged` keeps an approved Chrome Web Store version until you publish it by hand.

Each run builds what it uploads in release mode, so a stale build is never uploaded.

## Credentials

| Store                  | Environment variables                                                                                                                            | Where to get them                                                                                       |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------- |
| Chrome Web Store       | `LEPTOSKIT_PUBLISH_CHROME_SERVICE_ACCOUNT`: the JSON key, or its path                                                                            | A Google Cloud service account, added in the developer dashboard under **Account**, **API access**      |
| Microsoft Edge Add-ons | `LEPTOSKIT_PUBLISH_EDGE_CLIENT_ID` and `LEPTOSKIT_PUBLISH_EDGE_API_KEY`                                                                          | Partner Center's **Publish API** page. Keys expire after 72 days.                                       |
| addons.mozilla.org     | `LEPTOSKIT_PUBLISH_FIREFOX_JWT_ISSUER` and `LEPTOSKIT_PUBLISH_FIREFOX_JWT_SECRET`                                                                | The add-on developer hub's **API keys** page                                                            |
| App Store Connect      | `LEPTOSKIT_PUBLISH_APPLE_ISSUER_ID`, `LEPTOSKIT_PUBLISH_APPLE_KEY_ID`, and `LEPTOSKIT_PUBLISH_APPLE_KEY`: the `.p8` file's path, or its contents | **Users and Access**, **Integrations**, **App Store Connect API**: a team key with the App Manager role |

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

The text the steps paste comes from a `store/` folder in your project, so it's reviewed like code:

```text
store/
├── listing.toml          privacy_policy, homepage, support, category
├── permissions.toml      one sentence per permission, for reviewers
└── en-US/
    ├── description.md
    └── release_notes.md  Edge's submission notes
```

```toml
# store/permissions.toml
activeTab = "Reads the page you're on, only when you press the toolbar button, so you can clip from it."
storage = "Keeps your sign-in and your chosen project on this device."
```

A permission without a sentence shows up in the steps as one to write. Store reviewers reject permissions they can't see a reason for more often than anything else.

## Let an agent finish the release

`leptoskit skill install` gives coding agents, such as Claude Code, leptoskit's skill for the version of the CLI you have. Its publishing guide walks an agent through a release: run the dry run and fix what fails, upload, then work through the steps in each dashboard with computer use. The agent stops for every step marked for the account holder, and never submits without being told to.

```bash
leptoskit skill install          # into .claude/skills and .agents/skills in this project
leptoskit skill install --global # for every project
leptoskit skill                  # print it, to paste into any agent
```

## What doesn't work yet

- **Google Play and notarized macOS downloads** aren't uploaded yet. Safari's extension ships inside your iOS app, so it goes through the App Store with the app.
- **Listing text and screenshots** aren't uploaded; they appear in the steps instead.
- **A brand new Firefox add-on** needs its first version submitted on the site. After that, `leptoskit publish firefox --submit` adds versions.
