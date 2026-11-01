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

## After a release

- `leptoskit publish` writes `.leptoskit/publish/next-steps.md` every run; it's the place to see what's left.
- Reviews take from hours to days. Look in the store's dashboard for their status, and don't resubmit while one is pending.
