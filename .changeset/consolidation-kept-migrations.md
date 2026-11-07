---
"release": patch
---

# Consolidation preserves kept migrations

`db migration consolidate --keep` now folds only the schema changes before the kept migrations. Previously, it included their changes in the consolidated migration as well, so applying the resulting history could try to add the same column twice. Keeping every draft leaves history unchanged, and an oversized keep list reports an error instead of panicking.
