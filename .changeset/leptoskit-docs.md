---
"release": minor
---

# Add `leptoskit_docs`, Markdown with leptos components

A new crate for writing documentation the way MDX does, with leptos in place of React: a capitalised tag in a Markdown file, such as `<Callout kind="tip">`, renders a leptos component registered by name, and the Markdown between its tags becomes the component's children. Lowercase tags are plain HTML, and GitHub alerts (`> [!TIP]`) render as callouts.

`Site` builds a whole book, laid out by an mdBook-style `SUMMARY.md`, into a static site: chapter navigation, an on-page table of contents, search, build-time syntax highlighting with tree-sitter grammars, light and dark themes, and copy buttons. Every link between pages and headings is checked, so a broken link fails the build instead of shipping. `Site::serve` rebuilds on every edit and reloads open pages.
