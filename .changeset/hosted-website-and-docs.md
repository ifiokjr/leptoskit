---
"release": minor
---

# Package the public website, console and documentation

The unpublished `leptoskit_app` serves a responsive LeptosKit landing page, a working shared component example, the console at `/console/`, and the full documentation book at `/docs/`. The website packager bundles the book, local fonts and the pinned Rive runtime. The app opens its deployment's documentation on web and native hosts. Billing returns to the console.

The original brand timeline uses existing Rive support with reduced-motion and visibility policies. Browser entrances progressively enhance visible content; general native Tailwind keyframe support is unchanged. `leptoskit_docs::Site::external_fonts(false)` supports local font stylesheets, and theme initialization uses an external same-origin script so the hosted book works under the broker's content security policy.
