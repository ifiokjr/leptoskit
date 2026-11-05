---
"release": minor
---

# Amend drafting between releases

`[db] draft = "amend"` makes `leptoskit db migration create` rewrite the newest unreleased migration from the snapshot before it instead of stacking a new one, so between releases there is one draft that keeps up with the tables — and the release's `migration consolidate` has one entry to fold or nothing to do. Data migrations (`--empty`) still stack, and a development database that applied the draft's earlier shape is told to run `db reset --yes` to come back in line.
