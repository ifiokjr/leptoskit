---
"release": minor
---

# Vector columns on pgvector and SQLite

The `db-vector` feature adds `leptoskit::db::vector`: `Vector<1536>` for embeddings, `HalfVector<N>` at half the size, `BitVector<N>` for binary quantisation, and `SparseVector<N>` for mostly-zero vectors on Postgres. The number of dimensions is part of the type, so a 768-dimension embedding can't be stored in, or searched against, a 1536-dimension column. Vectors travel as JSON (floats as arrays of numbers, bits as a string of `0` and `1`), and vector columns are private unless they say `#[db(public)]`.

`#[db(vector(distance = cosine, index = hnsw))]` declares how a column is measured: `l2`, `cosine`, `inner_product`, `l1`, `hamming`, or `jaccard`. `query().nearest(|a| a.embedding, &query_embedding).with_distance().limit(10)` returns the nearest rows first, with the table's filters, policies, and soft deletes applied. Postgres uses pgvector, created by the first migration that needs it, with HNSW or IVFFlat indexes for the column's distance, scanned until enough rows match the filters; `.exact()` skips them. SQLite needs nothing installed: sqlite-vec is compiled into the build, each indexed column gets a `vec0` table kept current by triggers, and every search is exact.

`query().hybrid(|h| h.text("passkeys in rust").vector(|a| a.embedding, &query_embedding))` ranks a searchable table by full-text relevance and vector similarity together, fused by reciprocal rank fusion.

A vector made from text columns, `source(title, body)`, records a fingerprint of them whenever it's written, so a write that changes only the text leaves it stale. `Article::stale_embeddings()` lists the rows to embed again, and `embed_stale(|a| a.embedding, &model, &db)` makes and stores their vectors with any model through the `Embedder` trait; none is wired in. `leptoskit db check` warns about vector columns clients can see, and, given a SQLite database, about tables past `[db.vector] exact_search_rows` rows.
