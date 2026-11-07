---
"release": minor
---

# A book's logo can lead back to its site

`Site::home` sets where the logo in a book's header leads, for a book that is one part of a larger site, such as `.home("/")` for a book under `/docs/` beside a landing page. Without it the logo still leads to the book's first page. The leptoskit.dev documentation uses it, so its logo now returns to the landing page.
