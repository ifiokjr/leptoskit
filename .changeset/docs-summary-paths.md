---
release: fix
---

# Keep documentation sources inside the book

Reject unsafe chapter paths in `SUMMARY.md`, including absolute paths, parent traversal, and Windows path prefixes or separators. Refuse summaries and chapters whose symlinks resolve outside the documentation source directory before rendering or replacing any output.
