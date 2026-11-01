# Releasing to the stores

`leptoskit publish` builds the app, uploads it to each store in `[publish]`, and ends with **What to do next**: every step the stores' APIs can't take, with the page to open and the text to enter. You finish a release by working through those steps.

Publishing has two halves on every store:

- **Upload** puts the build where the store keeps it as a draft. It reaches nobody.
- **Submit** (`--submit`) sends it for review or release. It reaches users. **Only do it when the person you're working for has said to release.**

## The release, step by step

1. **Check.** Run `leptoskit publish --dry-run`. It builds every package and runs every check without contacting a store. Fix what fails, then run it again.
2. **Credentials.** Run `leptoskit publish credentials`. It lists, for each configured store, which environment variables are set and which are missing, and where each one is made. **Don't create credentials yourself:** making an API key or a service account needs the account holder, two-factor sign-in, and sometimes payment. Ask the person to create the missing ones and set them. Never ask them to paste a secret into the conversation.
3. **Upload.** Run `leptoskit publish <stores>`, such as `leptoskit publish chrome firefox edge` or `leptoskit publish app_store`. Then read `.leptoskit/publish/next-steps.json`. The app stores number each build one more than the store's highest; `--build-number <n>` overrides it.
4. **Work through the steps.** Each step has:
   - `store`: which store;
   - `by`: `agent` or `person`;
   - `what`: one sentence;
   - `url`: the page to open;
   - `enter`: labels and values to paste;
   - `why`: why the API can't do it;
   - `done_when`: how you'll know.

   For each `agent` step, follow "Computer use in a store's dashboard" below. For each `person` step, stop and tell the person exactly what it asks of them, with its `url`, then wait until they say it's done.
5. **Run it again.** `leptoskit publish <stores>` again uploads the same version (or tells you it must be newer) and prints what is still left.
6. **Submit, when told to.** Ask the person whether to release. Only with their yes, run `leptoskit publish <stores> --submit`. In a terminal it asks once more; `--yes` skips that for scripts.

## Listings and screenshots

- Listing text lives in `store/`: `listing.toml` (name, keywords, addresses, category), `en-US/summary.md`, `en-US/description.md`, and `en-US/release_notes.md`. Each upload sends what the store's API takes; a step asks for the rest. When a step's value says `(write it in …)`, ask the person for the text and add it to that file rather than typing it into the dashboard, so the next upload carries it.
- `leptoskit publish screenshots` captures every store's screenshots into `store/screenshots/<size>/`, from `store/screenshots/screenshots.toml`. Browser sizes need `leptoskit dev` running; simulator and Android sizes need the app installed and Appium. Run `--dry-run` first to see the files. Name each size after the store size it is (`iphone-6.3`, `ipad-13`, `android-phone`, `android-tablet-7`, `android-tablet-10`, `play-feature-graphic`, `chrome-1280x800`, and the others in the publishing guide); `leptoskit publish` refuses a folder it doesn't know. Each upload to the App Store, Google Play, and addons.mozilla.org replaces that store's screenshots with the folders' files, after checking their sizes and counts (also under `--dry-run`); fix a refused image and run it again rather than uploading by hand. The Chrome Web Store and Edge have no image API: when their step asks for screenshots, upload the files from the folder it names.

## Computer use in a store's dashboard

When you have computer use (screenshots, clicks, and typing) or a browser automation tool:

- **Open the step's `url`.** If the page asks you to sign in, or for a two-factor code, stop: that is the person's to do. Ask them to sign in in that browser, then continue.
- **Screenshot first.** Before each change, take a screenshot and find the field by its visible label, matching the step's `enter` labels.
- **Paste exactly.** Enter each `value` as written, with no rewording. Values in parentheses, such as `(write one sentence for …)`, mean the project is missing that text: stop and ask the person for it, then add it to the file the value names (such as `store/permissions.toml`), so the next run has it.
- **Upload the files a step names**, from the paths it gives.
- **Save, then check.** Press the page's save button, take a screenshot, and confirm the change stuck before going on.
- **Stop for anything the step didn't describe:** an agreement to accept, a payment, a tax form, a legal declaration, a question about the app's content or audience, or a prompt you don't understand. Describe it to the person and wait.
- **Never press the store's own submit or publish button.** Releasing goes through `leptoskit publish --submit`, after the person says so.

## Each store's dashboard

**Chrome Web Store**

- Dashboard: <https://chrome.google.com/webstore/devconsole>.
- The item is created once there, which needs the account holder. That includes the one-time developer registration fee.
- Pages:
  - **Store listing:** description, category, language, a 128×128 icon, and at least one 1280×800 or 640×400 screenshot.
  - **Privacy practices:** the single purpose, one justification per permission, the data usage disclosures and their certification (a person step), and the privacy policy's address.
  - **Distribution:** visibility and regions.
- Common rejections:
  - a permission without a justification, or one the code doesn't use;
  - a broad host permission where `activeTab` would do;
  - a description that doesn't match what the extension does;
  - missing privacy policy for an extension that handles personal or website data.

**Microsoft Edge Add-ons**

- Dashboard: Partner Center, <https://partner.microsoft.com/dashboard/microsoftedge/overview>.
- Pages:
  - **Properties:** category, privacy policy, website, and support.
  - **Store listings:** description and screenshots, per language.
  - **Availability.**
- API keys expire after 72 days; an expired key is a person step to replace.

**addons.mozilla.org (Firefox)**

- Developer hub: <https://addons.mozilla.org/developers/>.
- The first version of a new add-on is submitted on the site: upload `.leptoskit/extension/firefox.zip`, and the source archive `.leptoskit/extension/firefox-source.zip` when asked for source code.
- Choosing a license there is a person step. After that, `leptoskit publish firefox --submit` creates versions.
- The listing (description, screenshots, privacy policy) is edited at `https://addons.mozilla.org/developers/addon/<id>/edit`.
- Common rejections:
  - source that doesn't rebuild to the same package (the archive's `BUILD.md` lists the exact commands);
  - a missing or wrong data collection declaration (`[extension.firefox].data_collection`).

**App Store Connect (iOS, with the Safari extension inside)**

- Dashboard: <https://appstoreconnect.apple.com/apps>. The app is created there once (**Apps**, **+**, **New App**), which needs the account holder; its Apple ID goes in `[publish.app_store].app_id`.
- `leptoskit publish app_store` needs a Mac with Xcode. It archives, uploads, waits for processing (up to an hour), and adds the build to `testflight_groups`. Testers in internal groups get it at once; external groups wait for Apple's beta review.
- Pages, under the app:
  - **Distribution, the version in preparation:** description, keywords, support URL, screenshots for each device size, and **App Review Information** (contact and demo account, a person step).
  - **App Information:** category, privacy policy URL, and the **age rating** questions (a person step).
  - **App Privacy:** the data collection answers (a person step).
  - **TestFlight:** a build marked **Missing Compliance** needs the export compliance answer (a person step). `ITSAppUsesNonExemptEncryption` in the app's `Info.plist` answers it for every build.
- Agreements, tax, and banking (**Business**) are the account holder's, and paid apps can't be submitted without them.
- Common rejections:
  - a demo account that doesn't work, or none when the app needs signing in;
  - screenshots that don't show the app in use;
  - a privacy policy URL that doesn't load, or App Privacy answers that don't match what the app collects;
  - a permission's usage description (`NS…UsageDescription`) that doesn't say why the app needs it.

**Google Play**

- Play Console: <https://play.google.com/console>. The app is created there once (**Create app**), which needs the account holder, and its **first App Bundle is uploaded there by hand**: `leptoskit publish play` says so and prints the bundle's path. After that, uploads go through the API.
- The service account is invited under **Users and permissions** with permission to release; the account holder does that (a person step).
- Pages, under the app:
  - **Main store listing:** name, short description (80 characters), full description, icon (512×512), feature graphic (1024×500), and at least two phone screenshots.
  - **App content:** the privacy policy, then **data safety**, **content rating**, **target audience**, and **ads**, which are person steps.
  - **Testing, Internal testing:** the testers' list, for the `internal` track.
- Common rejections:
  - a data safety form that doesn't match what the app or its SDKs collect;
  - a target SDK below Google Play's minimum (the command checks it);
  - a privacy policy URL that doesn't load;
  - permissions the app doesn't explain in its listing.

**macOS downloads (Developer ID)**

- No dashboard: `leptoskit publish macos` signs, notarizes, and staples, and writes `.leptoskit/publish/macos/<Name>.dmg` and `.zip`.
- It needs the **Developer ID Application** certificate in the Mac's keychain, which only the account holder can create (a person step), and the App Store Connect API key for notarization.
- If notarization fails, run the `xcrun notarytool log <id>` command the error prints, with the same key, and fix what it lists; usually an unsigned binary or a missing secure timestamp.
- With `[publish.macos].github_release`, `--submit` attaches the files to the GitHub release `v<version>` with `gh`. Ask before it: a release reaches everyone who downloads from it.

## Releasing from CI

- `.github/workflows/release.yml` (from `leptoskit new`, or `leptoskit publish workflow`) runs `leptoskit publish` on a `v<version>` tag, and with `--submit` when run by hand with **submit** checked.
- Adding the repository secrets is the person's to do: they are the credentials. Tell them the names from `leptoskit publish credentials`, plus `LEPTOSKIT_ANDROID_KEYSTORE_BASE64`, `LEPTOSKIT_MACOS_CERTIFICATE_BASE64`, and `LEPTOSKIT_MACOS_CERTIFICATE_PASSWORD` for the files.
- A release is: raise `app.version`, commit, then `git tag v<version>` and push the tag. The run's summary has "What to do next".

## After a release

- `leptoskit publish` writes `.leptoskit/publish/next-steps.md` every run; it's the place to see what's left.
- Reviews take from hours to days. Look in the store's dashboard for their status, and don't resubmit while one is pending.
