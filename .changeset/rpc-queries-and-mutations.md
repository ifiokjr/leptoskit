---
"release": minor
---

# Queries and mutations as operation kinds

`leptoskit_rpc` gains operations that every platform calls the same way, by name, with their arguments as one value:

- **`#[rpc::query]` and `#[rpc::mutation]`.** An `async fn` taking `cx: &rpc::QueryCx` or `cx: &rpc::MutationCx` becomes a `PascalCase` struct of its arguments on every target, implementing `rpc::Operation`. Only a server build keeps the body, so a native guest carries the struct and the client, and no server code.
- **Contexts.** `QueryCx::db()` reads as the caller, read-only by type, and records what it reads. `MutationCx::tx()` is one transaction, `SERIALIZABLE` on Postgres, run again on serialization failures and deadlocks (five retries by default, `retries = N`), with `cx.after_commit(…)` for work that must wait for the commit.
- **Receipts.** A mutation called with an `Idempotency-Key` runs once: the new `rpc` schema pack's `rpc.receipts` table keeps each caller's result.
- **Serving and calling.** `rpc::server::Server` answers `POST /api/<name>`, identifying callers through the new `rpc::Identify` trait, which `leptoskit_auth` implements behind its `rpc` feature. `rpc::call(&db, Op { … })` runs one from server code, and `rpc::client::call(Op { … })` from the browser, a native guest, or in process while rendering, with `use_rpc_query`, `query_scope` and `mutation_scope` for `leptoskit_query`.
- **One spec.** `#[api]` publishes them too. Functions in the spec gain a `kind` (`call`, `query` or `mutation`; spec version 2, which still reads version 1), OpenAPI marks them with `x-leptoskit-kind`, and every generated client takes an idempotency key for mutations.
- **Facade.** `rpc` now includes the client; `web`, `guest`, `server`, `db-server`, `db-postgres` and `db-sqlite` turn on the matching `leptoskit_rpc` features.
- **`leptoskit_db`.** `Db::begin_serializable` and `DbError::is_contended`.
