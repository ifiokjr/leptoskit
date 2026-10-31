# Publishing to the stores

Getting an app into a store is where a release usually loses a day. Every store has its own upload API, its own version rules, its own review form, and pages that can only be filled in by hand. `leptoskit publish` does the uploading, and finishes by telling you exactly what is left in each store's dashboard, with the text to paste.

Today it publishes [browser extensions](../platforms/extension.md) to the Chrome Web Store, Microsoft Edge Add-ons, and addons.mozilla.org.

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
  - on addons.mozilla.org, a validated upload.

  Nobody sees it yet.
- **Submit.** `--submit` sends it for review, and then to users. In a terminal, it asks once more first; `--yes` skips the question for scripts, and CI never asks. `--staged` keeps an approved Chrome Web Store version until you publish it by hand.

Each run builds the extension in release mode, so a stale build is never uploaded.

## Credentials

| Store                  | Environment variables                                                             | Where to get them                                                                                  |
| ---------------------- | --------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Chrome Web Store       | `LEPTOSKIT_PUBLISH_CHROME_SERVICE_ACCOUNT`: the JSON key, or its path             | A Google Cloud service account, added in the developer dashboard under **Account**, **API access** |
| Microsoft Edge Add-ons | `LEPTOSKIT_PUBLISH_EDGE_CLIENT_ID` and `LEPTOSKIT_PUBLISH_EDGE_API_KEY`           | Partner Center's **Publish API** page. Keys expire after 72 days.                                  |
| addons.mozilla.org     | `LEPTOSKIT_PUBLISH_FIREFOX_JWT_ISSUER` and `LEPTOSKIT_PUBLISH_FIREFOX_JWT_SECRET` | The add-on developer hub's **API keys** page                                                       |

Set them in the environment, or put them in a TOML file outside the project and pass `--credentials <file>`. A file inside the project is refused unless git ignores it. Values are never printed, in output or in errors.

## What to do next

Some steps have no API: the Chrome Web Store's listing and privacy practices, Edge's store listing, and the first version of a new Firefox add-on. Every run, including `--dry-run`, ends with them, store by store:

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

- **App stores:** the App Store, Google Play, and notarized macOS downloads aren't uploaded yet. Safari's extension ships inside your Apple app, so it goes through the App Store with the app.
- **Listing text and screenshots** aren't uploaded; they appear in the steps instead.
- **A brand new Firefox add-on** needs its first version submitted on the site. After that, `leptoskit publish firefox --submit` adds versions.
