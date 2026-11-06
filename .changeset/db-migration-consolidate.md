---
"release": minor
---

# Migration consolidation at releases

`leptoskit db migration consolidate` folds every migration after the latest release marker into one, written from a fresh diff of the release's baseline to the current tables: add-then-drop vanishes, rename chains collapse, and intermediate SQLite table rebuilds never happen. The consolidated migration carries a `replaces.json` naming what it folded, and every database converges on its own — one that already applied the folded migrations records the consolidated one without running it, while a fresh one runs it once; a database that applied only part of the set is refused by name. A migration whose SQL no longer matches what its snapshots generate — a data migration, or one written by an older leptoskit — is refused unless it is named in `--keep`, which leaves it and the migrations after it in place, after the consolidated one.
