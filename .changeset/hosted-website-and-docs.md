---
"release": minor
---

# Serve the docs book under a strict CSP

Two `leptoskit_docs::Site` changes let you serve the book from your own site:

- `Site::external_fonts(false)` stops the book loading its fonts from Google Fonts, for a site that supplies local fonts in its own stylesheet or has to work offline.
- The script that applies a reader's saved light or dark theme is now a same-origin file instead of an inline script, so the book works under a content security policy that forbids inline scripts.

The LeptosKit website is built this way: it serves the landing page, a live shared component example, the deployment console at `/console/`, and the whole book at `/docs/`, with local fonts and the Rive runtime packaged alongside. The management app's **Documentation** button opens that book on the web, iOS, and Android.
