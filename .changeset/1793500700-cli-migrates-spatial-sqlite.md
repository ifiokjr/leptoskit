---
"release": patch
---

# `lk db migrate` handles SQLite spatial columns

The CLI applies SQLite migrations in its own process, and its database layer lacked the `geo` feature, so a migration that adds a spatial column failed with "no such function: lk_geo_check". The CLI now registers the spatial functions as it does the vector ones.
