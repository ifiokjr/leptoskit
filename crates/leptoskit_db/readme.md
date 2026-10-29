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

## Queries and writes

With the `postgres` or `sqlite` feature, `Db::connect("postgres://…")` (or `sqlite://app.sqlite3`, or `sqlite::memory:`) opens a pool, and every table gets typed queries and writes:

```rust,ignore
let ada = NewUser { handle: "ada".into(), … }.insert(&db).await?;
let user = User::get(&db, ada.id).await?;
let recent = Post::query()
	.filter(|p| p.author_id.eq(user.id) & p.published_at.is_not_null())
	.order_by(|p| p.published_at.desc())
	.limit(20)
	.all(&db)
	.await?;

db.transaction(async |tx| {
	let tag = NewTag { name }.upsert(tx, |t| t.name).await?;
	NewPostTag { post_id, tag_id: tag.id }.insert(tx).await?;
	Ok::<_, DbError>(())
})
.await?;
```

Writes validate first, `save` bumps `updated_at`, and `delete` soft-deletes tables with a `#[db(deleted_at)]` column, which queries then leave out unless they ask `with_deleted()`. Upserts conflict on any unique index, partial ones included. `DbError` tells a missing row, a taken unique value, a broken foreign key, and a failed check apart, and never puts SQL or values in its message.

Postgres runs through `tokio-postgres`, with rustls TLS unless the URL says `sslmode=disable`; values bind to whatever type each placeholder has. SQLite runs through `rusqlite`, the same bundled SQLite as the on-device plugin, on tokio's blocking pool.

## Migrations

`leptoskit db` writes each migration to its own directory under `migrations/` and lists them, in order, in `migrations/registry.txt`. The server embeds them and applies the pending ones at startup:

```rust,ignore
static MIGRATIONS: &[Migration] = leptoskit::db::migrations!("../migrations");

let db = Db::connect(&database_url).await?;
db.migrate(MIGRATIONS).await?;
```

Each migration runs in its own transaction and is recorded, with a SHA-256 of its SQL, in `_leptoskit_migrations`. Editing a migration that has been applied stops the server rather than being skipped, and so does a database with a migration the build doesn't know. On Postgres an advisory lock lets several servers start at once; on SQLite, migrations that rebuild tables run with foreign keys off and check every foreign key before committing. The build fails if a migration directory isn't in the registry, or the registry names one that's missing.

For tests, `db::testing::sqlite()` is an in-memory database with every table created from the code, and `db::testing::sqlite_migrated(MIGRATIONS)` applies the real migrations instead.
