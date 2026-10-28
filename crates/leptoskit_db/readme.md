<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_db

> leptoskit's database layer: tables declared as Rust structs, typed IDs, and client-safe public projections.

Declare a table once, with `#[db::table]`:

```rust,ignore
/// Someone with an account.
#[db::table(name = "users", public)]
pub struct User {
	pub id: Id<User>,
	#[db(unique(except = deleted_at), max_len = 32)]
	pub handle: String,
	#[db(private)]
	pub email: String,
	#[db(secret)]
	pub password_hash: String,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}
```

- **On the server** (the `server` feature), that is the `User` row, with `Debug` that redacts `secret` columns and no serde impls, so a row can never be returned to a client; `NewUser`, everything an insert provides, whose `into_row` fills in the key and timestamps; and the table's schema, registered so that `leptoskit db` can turn every table into migrations.
- **Everywhere**, `PublicUser` is every column except the `private` and `secret` ones: serializable, described by JSON Schema (so generated RPC clients know its rules), and validated by `validate()` before a form is sent.
- **On the client**, `User` is an empty enum: `Id<User>` still names the table, but no row can be built or read. Client builds pull in no database driver or schema code.

Columns declare their rules once (`min_len`, `max_len`, `min`, `max`, `one_of`, `regex`) and get them in the database as checks, in Rust as `validate()`, and in JSON Schema. `Id<T>` columns are foreign keys to `T`, `#[derive(db::Enum)]` stores a unit enum as text, and `Json<T>` stores any serde value as JSON.
