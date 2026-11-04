---
"release": patch
---

# Bound SQLite memory and fix secure storage and clipboard

- The SQLite plugin stops a statement whose working memory grows by more than 64 MiB, and refuses temporary tables, views, indexes and triggers. Before, a `DISTINCT` over generated values, or temporary tables filled one call at a time, could exhaust the app's memory, because neither counted against the page quota.
- `SecureStorage` stores keys it hashes under a `sha256-` prefix. Before, the hex digest of a key such as `auth/token` was itself a readable key, so the two read and wrote the same secret.
- The Android clipboard plugin reads only an item's text. It no longer opens a content URI that another app placed on the clipboard, which read that app's provider on the UI thread with no size limit.
