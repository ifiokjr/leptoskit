---
"release": minor
---

# Declare database tables as Rust structs

`leptoskit_db` adds `#[db::table]`: one struct declares a table, its server-side row, the `New…` type an insert takes, and the `Public…` type a client may see, which leaves out every `private` and `secret` column. Rows have no serde impls and redact secrets in `Debug`, so they can't be returned from a server function by mistake; on the client the row type is empty, and no database code is compiled in. Columns declare rules once (`min_len`, `max_len`, `min`, `max`, `one_of`, `regex`) and get them as database checks, as `validate()` on the server and the client, and as JSON Schema for generated clients. `Id<T>` is a typed UUIDv7 key whose columns become foreign keys, `#[derive(db::Enum)]` stores enums as text, and every table registers its schema for the migrations `leptoskit db` will generate.
