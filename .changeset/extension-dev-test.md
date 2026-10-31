---
"release": minor
---

# Develop, test, and scaffold browser extensions

- **`leptoskit dev`** runs the extension beside the app's other platforms, in Chrome for Testing, and reloads it after every rebuild. `--platform extension` runs it alone.
- **`leptoskit test extension`** builds the extension and checks it in Chrome for Testing through chromedriver: every page surface shows something, the worker runs, and each content script puts its frame on a page with a strict content security policy.
- **`leptoskit new --extension`** (and `generate app --extension`) scaffolds an app with a popup and a background worker.
- **Plugins** declare the extension permissions their browser side needs with `extension-permissions` in `[package.metadata.leptoskit.runtime]`; the notifications and clipboard plugins do.
- **`leptoskit doctor`** reports Chrome for Testing for projects with an extension.
