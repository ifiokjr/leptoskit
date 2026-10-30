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

Many-to-many links are tables too: `#[db::link(name = "post_tags", from = Post as tags, to = Tag as posts)]` on a struct of the link's own columns gives `post.tags().add(&db, tag.id, …)`, `.all(&db)` for each tag with its link, and `tag.posts().count(&db)`. A shape fetches a row and the rows linked to it in one statement, with every table's policies applied at every level:

```rust,ignore
#[derive(db::Shape, Serialize)]
#[db(from = User, public)]
struct ProfileCard {
	handle: String,
	#[db(count(posts))]
	post_count: i64,
	#[db(via = posts, order_by = |post| post.created_at.desc(), limit = 3)]
	latest: Vec<PostTitle>,
}

let card: ProfileCard = User::query().filter(|u| u.handle.eq(handle)).shape().one(&db).await?;
```

`projection(SelfUser)` on a table, with `#[db(private, in(SelfUser))]` on a column, declares a second client type that shows that private column, for an audience such as the signed-in user.

Postgres runs through `tokio-postgres`, with rustls TLS unless the URL says `sslmode=disable`; values bind to whatever type each placeholder has. SQLite runs through `rusqlite`, the same bundled SQLite as the on-device plugin, on tokio's blocking pool.

## Live tables and change capture

Mark a table `live` when an open screen should see its writes. Every write to it — from the ORM, raw SQL, or another process — inserts a row into `live.changes` in the writing transaction, and every committing transaction gets one version in `live.commits`:

```rust,ignore
#[db::table(name = "messages", live)]
pub struct Message {
	pub id: Id<Self>,
	pub channel_id: Id<Channel>,
	pub body: MessageBody,
	/// Wide data nobody needs in a change notification.
	#[db(live = skip)]
	pub preview: Vec<u8>,
	#[db(created_at)]
	pub created_at: Timestamp,
}
```

The macro implements `db::LiveTable` and `LIVE_COLUMNS`: every column except `secret` ones and those marked `#[db(live = skip)]`. A query on a read-only `LiveDb` (the RPC's live context) needs `LiveTable`, so forgetting `live` is a compile error at the query rather than a stale screen.

`db.changes().since(version).all()` reads the committed changes in version order, with the row before and after (without secret or skipped columns) and the names of the columns an update changed. `db.prune_live(retention)` deletes changes and commits outside the resume window. On Postgres `live.seal()` runs at commit under a transaction-scoped advisory lock and sends `NOTIFY leptoskit_live`; on SQLite the transaction stamps its changes just before it commits. The RPC's live hub is the first consumer.

`leptoskit db check --live` checks a connected database's capture triggers, functions, and sequences against the schema, and warns about a wide column copied into the change log.

## Migrations

`leptoskit db` writes each migration to its own directory under `migrations/` and lists them, in order, in `migrations/registry.txt`. The server embeds them and applies the pending ones at startup:

```rust,ignore
static MIGRATIONS: &[Migration] = leptoskit::db::migrations!("../migrations");

let db = Db::connect(&database_url).await?;
db.migrate(MIGRATIONS).await?;
```

Each migration runs in its own transaction and is recorded, with a SHA-256 of its SQL, in `_leptoskit_migrations`. Editing a migration that has been applied stops the server rather than being skipped, and so does a database with a migration the build doesn't know. On Postgres an advisory lock lets several servers start at once; on SQLite, migrations that rebuild tables run with foreign keys off and check every foreign key before committing. The build fails if a migration directory isn't in the registry, or the registry names one that's missing.

For tests, `db::testing::sqlite()` is an in-memory database with every table created from the code, and `db::testing::sqlite_migrated(MIGRATIONS)` applies the real migrations instead.
