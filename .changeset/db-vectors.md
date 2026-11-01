---
"release": minor
---

# Vector columns on pgvector and SQLite

The `db-vector` feature adds `leptoskit::db::vector`: `Vector<1536>` for embeddings, `HalfVector<N>` at half the size, `BitVector<N>` for binary quantisation, and `SparseVector<N>` for mostly-zero vectors on Postgres. The number of dimensions is part of the type, so a 768-dimension embedding can't be stored in a 1536-dimension column. Vectors travel as JSON: floats as arrays of numbers, bits as a string of `0` and `1`.

Migrations know vector columns: on Postgres they're pgvector's `vector`, `halfvec`, `bit`, and `sparsevec`, created by the first migration that needs the extension, with HNSW or IVFFlat indexes for the column's distance; on SQLite a `BLOB` checked for its length, and a sqlite-vec `vec0` table kept current by triggers. A vector made from text columns records a fingerprint of them whenever it's written, so a write that changes only the text leaves it stale.
