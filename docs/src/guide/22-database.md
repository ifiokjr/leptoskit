# The database

Most apps keep their data in a database, and three things go wrong with it again and again. The tables in the code drift from the tables in the database. Migrations get written by hand and break on the one machine that had different data. And a row with a password hash or an email address ends up serialized straight into a response.

leptoskit's database layer answers each one. You declare the tables as Rust structs. `leptoskit db` generates the migrations by comparing those structs with the last migration. Every table also gets a `Public…` type, which leaves out the private columns and is the only form of a row a client ever receives. Postgres is the default; SQLite works the same way.

## Start with a database

```sh
leptoskit new my_app --db postgres   # or --db sqlite
```

Next to the usual crates, the scaffold adds:

- `my_app_db`, the crate that declares the tables, with an example `Note` table;
- `migrations/`, empty until you create the first migration;
- a `[db]` table in `leptoskit.toml`;
- a server that applies pending migrations before it serves the first request;
- a backend that serves `GET /api/notes` and `POST /api/notes`.

Create the first migration, then run the app:

```sh
leptoskit db migration create init
leptoskit dev
```

A Postgres app reads its database from `DATABASE_URL`, such as `postgres://localhost/my_app`. A SQLite app uses `my_app.sqlite3` in the directory it starts from, unless `DATABASE_URL` names another database.

## Declare a table

A table is a struct with `#[table]`. Each field is a column, and `#[db(...)]` adds the rules the database enforces:

```rust
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::table;

/// Someone with an account.
#[table(name = "users", public)]
pub struct User {
	pub id: Id<User>,
	#[db(unique, min_len = 3, max_len = 32, regex = "^[a-z0-9_]+$")]
	pub handle: String,
	#[db(private)]
	pub email: String,
	#[db(secret)]
	pub password_hash: String,
	#[db(min = 0, default = 0)]
	pub karma: i32,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// Something a user wrote.
#[table(name = "posts", public, index(author_id))]
pub struct Post {
	pub id: Id<Post>,
	#[db(on_delete = cascade)]
	pub author_id: Id<User>,
	#[db(min_len = 1, max_len = 200)]
	pub title: String,
	#[db(one_of("draft", "published"), default = "draft")]
	pub status: String,
}

leptoskit_db::export_schema!();
```

That declares:

- `User`, the full row, which exists only on the server;
- `NewUser`, to insert one, without the ID or the timestamps, which the database fills in;
- `PublicUser`, which leaves out `email` and `password_hash` and is serializable. `public` on the table is what generates it;
- `UserColumns`, the typed columns that queries use.

IDs are UUIDv7 values, typed by their table: an `Id<User>` can't be passed where an `Id<Post>` is expected, and a field of type `Id<User>` is a foreign key to `users`.

The rules come in four kinds:

- **Text:** `min_len`, `max_len`, `regex`, `one_of(...)`.
- **Numbers:** `min`, `max`.
- **Keys and indexes:** `unique`, `key`, `index(...)`, `on_delete`.
- **Timestamps:** `created_at`, `updated_at`, `deleted_at`.

The rules are checked twice. Rust validates a row before it's written, so the error names the column. The database checks again with constraints, so a row written another way can't break them.

`export_schema!()` lets `leptoskit db` read the tables. It does nothing in a normal build.

## Private and secret columns

`#[db(private)]` leaves a column out of the `Public…` type. Use it for what only the server needs, such as an email address. `#[db(secret)]` does the same, and also hides the value from the row's `Debug` output, so a password hash never reaches a log line.

A client compiles only the `Public…` types. `User` itself is an empty type in the browser and in native apps. `Id<User>` still works there, but nothing on a client can build or read a full user. Return `PublicUser` from the server:

```rust
pub async fn profile(
	Extension(db): Extension<Db>,
	Path(id): Path<Id<User>>,
) -> Result<Json<PublicUser>, StatusCode> {
	let user = User::get(&db, id).await.map_err(status)?;
	Ok(Json(user.into()))
}
```

`status` turns a `DbError` into an HTTP status, as in the scaffold's `rpc` module.

## Values that are valid by construction

A handle shows up in many places: the users table, a mention in a comment, a form, a URL. Repeating `min_len = 4, max_len = 20, regex = …` on every column is how one of them ends up checked differently. Declare the rules once, on a type:

```rust
use leptoskit_db::Scalar;

/// A public handle.
#[derive(Scalar, Clone, Debug, PartialEq, Eq, Hash)]
#[db(min_len = 4, max_len = 20, regex = "^[a-z0-9_]+$", normalize(trim, lowercase))]
pub struct Handle(String);

let handle = Handle::parse(" Ifiok_Jr ")?;   // "ifiok_jr"
assert!(Handle::parse("no").is_err());
```

A scalar can only be made through `parse`, which normalizes the value and then checks it, so a `Handle` that exists is valid. Use it as a column type, and every column of that type gets the rules as database checks:

```rust
#[table(name = "users", public)]
pub struct User {
	pub id: Id<User>,
	#[db(unique)]
	pub handle: Handle,
	#[db(private)]
	pub email: Email,
}

User::query().filter(|user| user.handle.eq(Handle::parse("Ada")?)).first(&db).await?;
```

The same type works on the client. `PublicUser` carries a `Handle`. Deserializing one goes through `parse`, so bad input never reaches a server function, and its JSON Schema carries the rules for generated clients. A form can call `Handle::parse` before it sends anything.

A scalar wraps a `String` or a number. Text scalars take `min_len`, `max_len`, `regex`, and `normalize` (`trim`, `lowercase`, `uppercase`, or several: `normalize(trim, lowercase)`). Number scalars take `min` and `max`:

```rust
#[derive(Scalar, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[db(min = 1, max = 5)]
pub struct Stars(i16);
```

Three come built in:

- `Email`, trimmed and lowercased. Its check is pragmatic: one `@`, no spaces, and a dot in the domain.
- `Url`, for `http` and `https` addresses.
- `Slug`, such as `hello-world-2`.

Postgres checks a `regex` in the database. SQLite leaves it to the app, and a row whose value breaks the pattern can't be read back as the scalar.

## Columns many tables share

Most tables want `created_at` and `updated_at`, and an order wants a billing address and a shipping address with the same columns. Declare a set of columns once with `#[db::mixin]`, and flatten it into any table:

```rust
/// A postal address.
#[db::mixin(index(city))]
pub struct Address {
	#[db(max_len = 100)]
	pub street: String,
	pub city: String,
	#[db(private)]
	pub notes: Option<String>,
}

#[table(name = "orders", public)]
pub struct Order {
	pub id: Id<Order>,
	pub total: i64,
	#[db(flatten, prefix = "billing_")]
	pub billing: Address,
	#[db(flatten, prefix = "shipping_")]
	pub shipping: Address,
	#[db(flatten)]
	pub times: db::Timestamps,
}

order.billing.city;                                         // fields live where you'd expect
Order::query().filter(|order| order.billing_city.eq("Lagos")); // columns keep flat names
```

The table gets the mixin's columns as if they were declared in place: `billing_street`, `billing_city`, and so on. A `prefix` lets one mixin appear twice. The mixin's own rules come along: its fields' `#[db(...)]` options, and the indexes and unique constraints on `#[db::mixin(...)]`.

What the other generated types hold:

- **`PublicOrder`** nests each mixin as its public type. Here that's `PublicAddress`, which leaves out `notes`.
- **`NewOrder`** takes a mixin whole when an insert provides all of its fields. A mixin the database layer fills in completely, such as `db::Timestamps`, is left out of the insert type. A mixin it fills in partly gets a `New…` struct of the remaining fields.

Three mixins come built in:

- `db::Timestamps`: `created_at` and `updated_at`.
- `db::SoftDelete`: `deleted_at`, so deleting the row hides it rather than removing it.
- `db::Versioned`: a `version` for optimistic concurrency. `save` writes the row only while its version is the one it read, then increments it, so two people editing the same row can't silently overwrite each other. The second save fails with `DbError::Stale`; read the row again and retry. Bulk updates increment the version too.

To write code once for every table with a mixin, use `HasMixin`:

```rust
fn created<T: db::HasMixin<db::Timestamps>>(row: &T) -> db::Timestamp {
	row.mixin().created_at
}
```

## Read and write

```rust
let ada = NewUser {
	handle: "ada".into(),
	email: "ada@example.com".into(),
	password_hash,
	karma: 0,
}
.insert(&db)
.await?;

let top = User::query()
	.filter(|user| user.karma.gte(10) & user.handle.contains("a"))
	.order_by(|user| user.karma.desc())
	.limit(20)
	.all(&db)
	.await?;

let mut user = User::get(&db, ada.id).await?;
user.karma += 1;
user.save(&db).await?;

User::update()
	.filter(|user| user.handle.eq("ada"))
	.set(|user| user.karma, 7)
	.run(&db)
	.await?;
```

Queries end in `all`, `first`, `one`, `count`, `exists`, `delete`, or `destroy`. `User::find` returns `Option<User>`, and `User::get` returns an error when no row matches. To insert a row, or update it when a unique column already has the value, use `upsert(&db, |user| user.handle)`.

A table with a `deleted_at` column deletes softly. `delete` sets the timestamp, queries skip those rows unless you add `with_deleted()`, and `destroy` removes the row for good.

Transactions commit when the closure returns `Ok` and roll back otherwise:

```rust
let post = db
	.transaction(async |tx| {
		let author = NewUser { /* ... */ }.insert(tx).await?;
		NewPost { author_id: author.id, title: "Hello".into(), status: "draft".into() }
			.insert(tx)
			.await
	})
	.await?;
```

`DbError` says what went wrong (`NotFound`, `Conflict`, `ForeignKey`, `Check`, or `Invalid`) and names the table, never the values, so passing it on with `?` can't leak data.

## Follow links

An `Id<T>` column links to `T`. Each link gets an accessor named after the field, without its `_id`, and a query for the rows pointing at a given row:

```rust
#[table(name = "posts", public)]
pub struct Post {
	pub id: Id<Post>,
	#[db(on_delete = cascade, backlink = posts)]
	pub author_id: Id<User>,
	pub editor_id: Option<Id<User>>,
	pub title: String,
}

let author: User = post.author(&db).await?;
let editor: Option<User> = post.editor(&db).await?;
let theirs = Post::by_author(user.id).order_by(|post| post.title.asc()).all(&db).await?;
```

`backlink = posts` adds the reverse to `User`: `user.posts()` is the same query as `Post::by_author(user.id)`, so it filters, orders, and counts like any other:

```rust
let latest = user.posts().order_by(|post| post.title.desc()).limit(5).all(&db).await?;
```

Links follow the same rule as every query: a soft-deleted row isn't there. A required link to one fails with `DbError::NotFound`, and an optional link returns `None`. A backlink can only be added to a table declared in the same crate, because Rust only allows a type's own crate to add methods to it. For a table from another crate, use `Post::by_author(user.id)`.

## Migrations

Each migration is a directory under `migrations/`. It holds `migration.sql` and `schema.json`, a snapshot of the tables after it runs. `registry.txt` lists the migrations in order. `leptoskit db migration create <name>` compares your tables with the latest snapshot and writes the SQL for the difference:

```console
$ leptoskit db migration create add karma
reading the tables in my_app_db
  + column users.karma (int32)
wrote migration 20261028T120000_add_karma in migrations/20261028T120000_add_karma
review migration.sql, then commit it with schema.json and registry.txt
```

It needs no database, so it gives the same result on every machine.

- **Renames.** When a column disappears and another appears, it asks whether that was a rename. Say so in the code to skip the question: `#[db(renamed_from = "handle")]`. Once the migration exists the hint does nothing, so remove it whenever you like.
- **Deleting data.** A step that would delete data, such as dropping a column, stops unless you pass `--allow-destructive`.
- **Data changes.** `--empty` writes a migration with no schema changes, for SQL you write yourself.
- **SQLite rebuilds.** Changes that SQLite can't make in place, such as adding a check, rebuild the table and copy its rows across.

Run `leptoskit db check` in CI. It fails when the tables changed without a migration.

The server applies migrations as it starts:

```rust
let db = leptoskit::db::Db::connect(&url).await?;
for name in db.migrate(my_app_db::MIGRATIONS).await? {
	println!("applied migration {name}");
}
```

`migrations!("../migrations")` embeds them in the binary, and the build fails if one is missing or unlisted. Each migration runs in its own transaction. It's recorded in `_leptoskit_migrations` with a checksum of its SQL. On Postgres, an advisory lock keeps two servers from migrating at once. If an applied migration's SQL has changed since, the server stops rather than guessing. Applied migrations are history: make the change in a new one.

The rest of the commands work against a database:

| Command                    | What it does                                                                                                 |
| -------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `leptoskit db status`      | Lists applied and pending migrations.                                                                        |
| `leptoskit db migrate`     | Applies pending migrations without starting the server.                                                      |
| `leptoskit db reset --yes` | Recreates a local database from the migrations, deleting its data. It refuses any database that isn't local. |

They read the URL from `DATABASE_URL`, or from `--url`.

## Configuration

```toml
[db]
dialect = "postgres" # or "sqlite"
crate = "my_app_db" # the crate that declares the tables; this is the default
features = ["server"] # the features that compile its server side
migrations = "migrations" # relative to the project root
url_env = "DATABASE_URL" # where status, migrate, and reset find the database
```

An empty `[db]` table uses every default.
