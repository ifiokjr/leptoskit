# The database

Most apps keep their data in a database, and three things go wrong with it again and again. The tables in the code drift from the tables in the database. Migrations get written by hand and break on the one machine that had different data. And a row with a password hash or an email address ends up serialized straight into a response.

leptoskit's database layer answers each one. You declare the tables as Rust structs. `leptoskit db` generates the migrations by comparing those structs with the last migration. Every table also gets a `Public…` type, which leaves out the private columns and is the only form of a row a client ever receives. Postgres is the default; SQLite works the same way.

## Start with a database

```sh
leptoskit new my_app --db          # Postgres
leptoskit new my_app --db sqlite   # SQLite
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

A Postgres app reads its database from `DATABASE_URL`. While you develop, `leptoskit dev` provides one, as the next section explains. A SQLite app uses `my_app.sqlite3` in the directory it starts from, unless `DATABASE_URL` names another database.

## A database while you develop

A Postgres app needs a Postgres server, and installing and configuring one is a chore that has nothing to do with your app. So when `DATABASE_URL` isn't set, `leptoskit dev` runs one for you and hands the app server its URL:

```console
$ leptoskit dev
[db] downloading Postgres 18 once into /home/ada/.cache/leptoskit/postgresql; later runs reuse it
[db] started Postgres at postgres://postgres:***@localhost:54123/my_app?sslmode=disable
[web] running the app server on http://127.0.0.1:3000
```

- **The first run** downloads Postgres once, about 100 MB, into `~/.cache/leptoskit/postgresql` (or `$XDG_CACHE_HOME/leptoskit/postgresql`). Every project on the machine reuses it.
- **Your data** lives in the project's `.leptoskit/db/postgres`, which the scaffold's `.gitignore` leaves out, so it survives restarts. Delete that directory to start from nothing.
- **The server** listens only on this machine, on a port chosen on the first run, with a password kept in the same directory. It stops when `leptoskit dev` does.
- **`leptoskit db`** uses the same database when `DATABASE_URL` isn't set: `leptoskit db migrate`, `status`, and `reset` start it for the command, or use it while `leptoskit dev` has it running.

If the project uses [devenv](https://devenv.sh) and its `devenv.nix` turns on `services.postgres`, `leptoskit dev` uses that server instead and downloads nothing. Run `leptoskit dev` inside `devenv shell`, with `devenv up` running, and it creates the app's database there if it's missing:

```nix
# devenv.nix
{ ... }: {
  services.postgres.enable = true;
}
```

### A database for each branch

Switching git branches with one development database mixes them up: a feature branch adds a column, and back on `main` the app fails on a table it doesn't expect. So on the development Postgres each branch gets its own database, copied from the main branch's, rows and all, the first time you use the branch:

```console
$ git switch -c feat/karma
$ leptoskit dev
[db] branch `feat/karma` has its own database, `my_app__feat_karma`, copied from `my_app`
```

`leptoskit dev` follows `git switch` while it runs: it stops the app server, moves to the branch's database, brings it in step with the tables, and restarts the server on it. `leptoskit db migrate`, `status`, `push`, and `reset` use the branch's database too. The main branch, `main` or `master` or whatever `origin` points at, keeps the app's database, and so does a detached `HEAD`.

Postgres copies a database only while nothing is connected to it, so stop a `psql` session on the main branch's database before using a new branch. `leptoskit db branch list` shows each branch's database, and `leptoskit db branch prune --yes` deletes those whose branch is deleted or merged into the main branch; without `--yes` it only says what it would delete. Set `branch_databases = false` under `[db]` to share one database across branches. A database you name with `DATABASE_URL`, and SQLite apps, are left as they are.

To use a database of your own, set `DATABASE_URL`, and `leptoskit dev` leaves it to you. To never start one, set `dev_server = "off"` under `[db]`. To skip the download, set `LEPTOSKIT_POSTGRES_BIN` to the `bin` directory of a Postgres you've installed; `leptoskit dev` runs that instead. SQLite apps need no server, so none of this applies to them.

### See the data in a browser

While you build a feature you want to look at the rows it writes, and check that someone who isn't signed in can't see a draft. A SQL shell shows every row, whatever your policies say. The studio shows the database as your app sees it:

```console
$ leptoskit db studio
leptoskit db studio: http://127.0.0.1:52311/?token=6f3c…
```

Open that address. The studio runs inside your db crate's tests, so it reads each table through its own policies, as the viewer you pick at the top of the page:

- **Nobody**: an anonymous request, the default.
- **A persona**: a kind of request with an identity you type. With `leptoskit::auth`, `account` views the data as a signed-in account, by its ID. Register your own with `register_persona!` for the globals your policies read.
- **`system`**: every row, bypassing policies. You type why, and the audit log records it.

It has pages for each table's rows, with paging, a filter, and sorting by any column; the schema the next migration would see; the migrations and which this database has; the SQL each table's policies add for the current viewer; the audit log; and the live changes the viewer hears of. Columns marked `secret` always show as `‹secret›`.

**Changing rows.** Edits appear only in edit mode, which the header turns on. Each change shows what it will do and waits for you to confirm it, then runs through the table's own `save` or `delete`, as the current viewer, with its hooks, rules, policies, and audit log. A change the viewer may not make is refused with the policy's message.

**Who can reach it.** It listens on `127.0.0.1` only, on a free port unless you pass `--port`, and answers only requests that carry the token printed for this run and name that address, so other machines and other sites in your browser can't use it. `leptoskit dev --studio` runs it beside the app.

**It never ships.** The studio exists only in builds with debug assertions: `leptoskit db studio` builds your db crate's tests with the `studio` feature of `leptoskit_db` (or `db-studio` of `leptoskit`), and a release build leaves all of it out, even with the feature on.

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

### Show private columns to the people they belong to

Someone looking at their own account should see their email address; nobody else should. With only `PublicUser`, the server either leaves the email out for everyone or writes a second struct by hand and copies fields into it, which drifts the first time a column changes. Declare the audience on the table instead, and mark which private columns it sees:

```rust
#[table(name = "users", public, projection(SelfUser))]
pub struct User {
	pub id: Id<User>,
	pub handle: String,
	#[db(private, in(SelfUser))]
	pub email: Email,
	#[db(private)]
	pub risk_score: i32,
	#[db(secret)]
	pub password_hash: String,
}

// The signed-in user's own account.
pub async fn me(db: Db, id: Id<User>) -> Result<SelfUser, DbError> {
	Ok(User::get(&db, id).await?.into())
}
```

`SelfUser` holds every public column, plus `email`. It is serializable, carries the same rules for forms and generated clients as `PublicUser`, and, like it, compiles on every platform, so the web, iOS, Android, macOS, Windows, Linux, terminal, and HarmonyOS clients all receive the same type. A table can declare several projections, `projection(SelfUser, Staff)`, and a column can be in several, `in(SelfUser, Staff)`. A `secret` column is never in one: it never leaves the server.

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

Two more come with the `solana` feature of `leptoskit_db`, which sign-in turns on: `Pubkey`, a Solana address such as `9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM`, and `Signature`. Both are base58 text and parse strictly: the text must decode to exactly 32 bytes for an address, or 64 for a signature, so an address that parses is one a wallet could hold. `to_bytes()` and `from_bytes()` convert to and from the raw bytes.

```rust
use leptoskit::db::Pubkey;

#[table(name = "vaults")]
pub struct Vault {
	pub id: Id<Vault>,
	/// Who may change the vault: whoever holds this wallet.
	pub treasury: Pubkey,
}
```

A rule can then say "the owner of this wallet", with the signed-in person's linked wallets; see [wallets in rules](23-sign-in.md#rules-about-the-signed-in-person).

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

## Columns the database computes

Handles should be unique whatever their case, so `Ada` and `ada` can't both sign up, and found the same way. Lowercasing in every query can't use an index, and storing a lowercase copy yourself means every write has to remember to update it. Let the database compute the copy instead:

```rust
#[table(name = "users", public)]
pub struct User {
	pub id: Id<User>,
	pub handle: String,
	#[db(generated = "lower(handle)", unique)]
	pub handle_key: String,
	#[db(generated(postgres = "char_length(handle)", sqlite = "length(handle)"))]
	pub handle_length: i32,
}

let ada = NewUser { handle: "Ada".into() }.insert(&db).await?;
assert_eq!(ada.handle_key, "ada");
User::query().filter(|user| user.handle_key.eq("ada")).one(&db).await?;
```

A generated column is SQL over the row's other columns, which the database stores with the row and recomputes on every write, so it can be indexed and unique like any other. Write one expression for both databases with `generated = "…"`, or one each with `generated(postgres = "…", sqlite = "…")` where their functions differ.

- **Nothing writes it.** `NewUser` has no `handle_key`. `insert`, `upsert`, and `save` return the row with the values the database computed, and `User::update().set(…)` on a generated column fails with `DbError::Encode`. Until the row is written, `NewUser::into_row()` holds the type's `Default` value, so a generated column's type must have one.
- **Reads see it.** Filters, orderings, shapes, and the `Public…` type read it like any column; mark it `private` to keep it on the server.
- **What it can't be.** A generated column can't be the key, a link, a timestamp, or have a `default` or rules such as `min_len`; put rules on the columns it's computed from. A schema pack's tables can't have one, since its SQL is written for particular databases.
- **Changing one.** To change how a column is computed, add a new column under another name and drop the old one: `leptoskit db migration create` refuses to change the expression in place, since the migration would also have to rebuild its indexes. Postgres won't change the type of a column while a generated column reads it, so drop the generated column in the migration that changes the type, which drops it first, and add it back in the next one.

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

## When a linked row is deleted

A link says what happens to its rows when the row they point at is deleted:

| `#[db(on_delete = …)]` | What happens                                                                                     |
| ---------------------- | ------------------------------------------------------------------------------------------------ |
| `restrict`             | The delete fails while rows point at it. The default for a required link.                        |
| `set_null`             | The link becomes `None`. The default for an optional link.                                       |
| `cascade`              | The rows pointing at it are deleted too.                                                         |
| `deferred_restrict`    | Like `restrict`, but checked when the transaction commits, so rows can be inserted in any order. |

These apply to real deletes, which the database carries out. A soft delete is an update the database knows nothing about, so ask for it on the link:

```rust
#[table(name = "members")]
pub struct Member {
	pub id: Id<Member>,
	#[db(on_soft_delete = cascade)]
	pub club_id: Id<Club>,
	#[db(flatten)]
	pub deleted: db::SoftDelete,
}

club.delete(&db).await?;                       // the club's members are soft-deleted too
Club::query().filter(…).delete(&db).await?;    // and for every club a bulk delete reaches
```

Soft-deleting a club soft-deletes its members at the same moment, then whatever cascades from them, and so on, all in one transaction. Inside your own transaction, it joins yours and rolls back with it. Rows already deleted keep their original time, which also ends cycles in a tree. Both tables need a soft-delete column for this.

## Many-to-many links

A post has many tags, and a tag is on many posts. An `Id<T>` column can't say that, so this needs a table in between whose rows each pair a post with a tag, and often say something about the pair, such as where the tag sits in the post's list. Declare that table with `#[db::link]`, and write only what the link itself holds:

```rust
/// A tag on a post.
#[db::link(name = "post_tags", from = Post as tags, to = Tag as posts)]
pub struct PostTag {
	pub position: i16,
	#[db(created_at)]
	pub added_at: Timestamp,
}

post.tags().add(&db, tag.id, PostTagProperties { position: 0 }).await?;

let tags: Vec<(Tag, PostTag)> = post.tags().order_by(|link| link.position.asc()).all(&db).await?;
let ids: Vec<Id<Tag>> = post.tags().ids(&db).await?;
let tagged = tag.posts().count(&db).await?;
let has_it = post.tags().contains(&db, tag.id).await?;

post.tags().remove(&db, tag.id).await?;

// Exactly these tags, in this order: the rest are removed.
post.tags().set(&db, [(rust, PostTagProperties { position: 0 }), (sql, PostTagProperties { position: 1 })]).await?;
```

The macro adds the two ends, `post_id` and `tag_id`, which together are the table's key, so a post holds each tag once. Adding the same tag again replaces the link's properties. Deleting a post or a tag for good deletes its links. `PostTagProperties` holds everything an insert would provide except the two ends; a link with nothing else on it takes `()`. Write it as `db::link`: a bare `#[link]` is Rust's own attribute for native libraries.

Narrow what you read with `filter(|tag| …)` on the linked rows and `filter_link(|link| …)` on the links' own columns, and page with `limit` and `offset`. Links come in the order `order_by` gives, or else in the order of the linked rows' keys.

- **Both ends must be visible.** A link is there only when the request may see both its ends: a soft-deleted tag, or a post the request's policies hide, takes its links with it. Adding a link to a row the request can't see fails with `DbError::NotFound`.
- **Links are a table.** A link takes `#[db::policy]` rules like any table, and every option of `#[db::table]`. To say who may tag a post, write the rule on the link.
- **A table can link to itself.** Name the two columns: `#[db::link(name = "follows", from = User as following, to = User as followers, from_column = "follower_id", to_column = "followed_id")]`.
- **One owner per row.** `exclusive` lets each `to` row be linked from one `from` row only, so a second post pinning the same tag fails with `DbError::Conflict`.
- **Both tables in one crate.** `post.tags()` and `tag.posts()` are methods on the linked tables, so, as with backlinks, both must be declared in the crate that declares the link. For others, `Links::<PostTag, _, _>::forward(post_id)` and `Links::<PostTag, _, _>::backward(tag_id)` do the same.
- **Replacing every link at once.** `set` adds, updates, and removes links in one transaction. It replaces only what the request sees: a link to a row it can't see stays, and filters narrow it, so `post.tags().filter(|tag| tag.kind.eq("topic")).set(…)` leaves other tags alone. If a policy keeps a link from being removed, the whole change fails with `DbError::Forbidden`.
- **A schema pack's links** are read like any other, but only the pack adds or removes them.

## A link to one of several tables

A budget can belong to a person or to a team. Two optional columns, `person_id` and `team_id`, let a budget belong to both or to neither, and every new kind of owner adds another column and another check. Instead, declare what can own a budget as an enum, one variant per table, and link to the enum:

```rust
/// Anything that can own a budget.
#[db::polymorphic(name = "actors", public)]
pub enum Actor {
	Person(Person),
	Team(Team),
}

#[table(name = "people", public, extends = Actor)]
pub struct Person {
	pub id: Id<Person>,
	pub name: String,
}

#[table(name = "teams", public, extends = Actor)]
pub struct Team {
	pub id: Id<Team>,
	pub title: String,
}

#[table(name = "budgets", public)]
pub struct Budget {
	pub id: Id<Budget>,
	#[db(backlink = budgets)]
	pub owner_id: Id<Actor>,
	pub name: String,
}

let budget = NewBudget { owner_id: team.id.upcast(), name: "Q3".into() }.insert(&db).await?;

match budget.owner(&db).await? {
	Actor::Person(person) => println!("{}'s budget", person.name),
	Actor::Team(team) => println!("the {} team's budget", team.title),
}

let everyone: Vec<Actor> = Actor::query().all(&db).await?;
let teams = Actor::query().only([ActorKind::Team]).limit(20).all(&db).await?;
let theirs = Actor::get(&db, budget.owner_id).await?.budgets().all(&db).await?;
```

The `match` must handle every variant, so adding one, say `Actor::Bot`, points the compiler at every place that has to deal with it. Each table that is a variant says so with `extends`, and the enum lists it; the build fails if either is missing.

- **Storage.** The enum declares a table of its own, `actors`, with one row per person or team: its key and its kind. A variant's key is also a key of that table, so `Id<Actor>` columns are ordinary foreign keys, and `Id<Person>::upcast()` turns a person's key into an actor's. Inserting a person inserts their `actors` row first, and removing them removes it after, in one transaction.
- **Reading through the enum** runs one query per variant table, each with that table's own policies and soft deletes, and merges the rows in key order: the order they were created in. `only`, `ids`, `after`, `descending`, `limit`, `offset`, `count`, and `with_deleted` apply across all of them. One key, through `Actor::find`, a link, or `id.downcast::<Team>()`, takes two queries: its `actors` row says which table to read.
- **Policies.** `#[db::policy]` rules below `#[db::polymorphic]` govern the `actors` table, and every variant inherits them: a person is visible only when their `actors` row is, and changing or removing one needs that row to allow it too. A rule on the enum sees `id` and `kind`, so `|actor, _| !actor.kind.eq(ActorKind::Bot)` hides every bot, whatever the bots table's own rules say. With `policies = "rls"`, Postgres enforces the inherited rules as well.
- **Soft deletes** stay in the variant table. A soft-deleted person disappears from `Actor::query()` and from links, but keeps their `actors` row, so whatever points at them still holds. `#[db(on_soft_delete = cascade)]` on an `Id<Actor>` column follows a soft delete of any variant.
- **Removing.** Removing a person removes their `actors` row too, so a budget that still points at them stops it, as any required link would.
- **Live changes.** `#[db::polymorphic(…, live)]` publishes a change to `actors` whenever a row of any kind is added or removed. Tag a list read through the enum with `table_tag("actors")` and each variant's table, so that edits to a person refetch it too.
- **On the client** `Actor` is a marker, like a table, and `PublicActor` holds each variant's public type, tagged with its kind: `{"kind": "team", "row": {…}}`. `ActorKind` names the kinds in both places, and `#[db(rename = "…")]` on a variant changes the kind it stores.
- **What it can't do yet.** A shape can't follow a link to an enum, since its rows come from several tables; load them with the link's accessor. Every variant must be declared in the crate that declares the enum.

## Fetch linked rows in one query

A profile page wants a user's handle, how many posts they've written, and their three latest posts, each with its tags. Fetching the user, then the posts, then each post's tags is one query per row, which gets slower with every post, and leaves you assembling the result by hand. A **shape** says what you want as a struct, and fetches all of it in one statement:

```rust
use leptoskit::db::Shape;

#[derive(Shape, Serialize, Deserialize)]
#[db(from = User, public)]
pub struct ProfileCard {
	pub handle: String,
	#[db(count(posts))]
	pub post_count: i64,
	#[db(via = posts, order_by = |post| post.created_at.desc(), limit = 3)]
	pub latest: Vec<PostTitle>,
	#[db(via = posts, filter = |post| post.pinned.eq(true))]
	pub pinned: Option<PostTitle>,
}

#[derive(Shape, Serialize, Deserialize)]
#[db(from = Post, public)]
pub struct PostTitle {
	pub id: Id<Post>,
	pub title: String,
	#[db(via = tags)]
	pub tags: Vec<TagName>,
}

#[derive(Shape, Serialize, Deserialize)]
#[db(from = Tag, public)]
pub struct TagName {
	pub name: String,
}

let card: ProfileCard = User::query()
	.filter(|user| user.handle.eq(handle))
	.shape()
	.one(&db)
	.await?;
```

Each field reads one thing:

- **A column**, by the field's name: `handle`. To use another name, say which column: `#[db(column = display_name)] pub name: String`.
- **A count**, `#[db(count(posts))]`, of the rows a link reaches, with an optional `filter`.
- **Linked rows**, `#[db(via = posts)]`, as another shape. `via` names any way from this table to another: a link column without its `_id` (`via = author`), a `backlink` (`via = posts`), or a `#[db::link]` method (`via = tags`). The field's type says how many: `Vec<_>` for all of them, `Option<_>` for the first, and the shape itself for the row a required link points at. A `Vec` takes `filter`, where several must all match, `order_by`, where later ones break ties, `limit`, and `offset`; an `Option` takes all but `limit`.

What the query returns follows the same rules as every other query:

- **Policies apply at every level.** A nested post is there only if the request may read it, and a count counts what the request may see, so a shape never reveals more than separate queries would.
- **Soft-deleted rows are left out** of nested rows and counts. A required link to one, such as the author of a post whose author was deleted, fails with `DbError::NotFound`, as `post.author(&db)` does. Add `with_deleted` to a field to include them: `#[db(via = posts, with_deleted)]` for an archive page, or `#[db(via = author, with_deleted)]` to keep showing a deleted author.
- **Nested rows always come in the same order:** `order_by`, then the linked table's key.

`public` on a shape checks, when it compiles, that it reads only public columns and nests only public shapes, so it's safe to return from a server function; without `public`, a shape can read `private` columns, for use on the server. A shape is an ordinary struct on clients, so derive `Serialize` and `Deserialize` and return it from a server function to the web or native app as you would `PublicUser`. On Postgres and SQLite alike, a shape is one statement, with each nested field a subquery; a shape reads at most 100 fields, so nest some in another shape if you need more.

## Who may see and change what

Every server function that touches drafts has to remember the same rule: a draft is only for its author. Forget it once, and someone reads another person's drafts. Write the rule on the table instead, and every query obeys it:

```rust
/// The signed-in author, which the request sets.
pub struct Viewer(pub Id<Author>);

#[table(name = "articles", public)]
#[db::policy(select, |article, cx| article.published.eq(true)
	| cx.global(|viewer: &Viewer| article.author_id.eq(viewer.0)))]
#[db::policy(insert, update, delete, |article, cx| cx.global(|viewer: &Viewer| article.author_id.eq(viewer.0)),
	message = "Only the author can change an article")]
#[db::policy(deny update, |article, _| article.locked.eq(true), message = "Locked articles can't change")]
pub struct Article {
	pub id: Id<Article>,
	pub author_id: Id<Author>,
	pub published: bool,
	pub locked: bool,
}
```

Give each request a `Db` that knows who it's for:

```rust
let db = db.with_context(Context::new().with(Viewer(session.author_id)));

let visible = Article::query().all(&db).await?;   // published, plus this author's drafts
```

A rule is `|row, cx| …`, a condition built the same way as a `filter`, that can read the request's globals. A global is any value you add to the `Context`, found by its type. `cx.global(|viewer: &Viewer| …)` builds a condition from one, and matches nothing when the request doesn't have it, so a signed-out request is never mistaken for an author.

How rules combine:

- **No policies:** a table with none is open to server code, as before.
- **Default deny:** once a table has a policy, an action is refused unless a rule allows it.
- **`deny` wins:** a matching `deny` overrides every allow, even an administrator's.
- **Silent filters:** reads, updates, and deletes skip the rows the request may not touch, so they simply aren't there. That includes counts and links.
- **Checked writes:** an insert, or an update whose result isn't allowed, fails with `DbError::Forbidden`, carrying the rule's `message`, and nothing is written. Policies check the row as stored, defaults included. An upsert can't take over a row its request may not change.

A `Db` made by `Db::connect` acts for an empty context, so a forgotten `with_context` refuses rather than reveals. Work no request asked for, such as migrations and nightly jobs, bypasses policies explicitly, and says why:

```rust
let admin = db.system("nightly purge");
```

Write `#[db::policy]` below `#[db::table]`, which reads it.

### Rules about linked rows

Many rules depend on other rows: a project is visible to the members of the team that owns it, and a team may have at most ten projects. `cx.count(query)` counts the rows a query finds, so a rule can compare that number:

```rust
/// The signed-in person, which the request sets.
pub struct Viewer(pub Id<Person>);

#[table(name = "projects")]
#[db::policy(select, insert, update, delete, |project, cx| cx.global(|viewer: &Viewer| {
	cx.count(Member::query().filter(|member| {
		member.team_id.eq(project.team_id) & member.person_id.eq(viewer.0)
	}))
	.gt(0)
}))]
#[db::policy(deny insert, |project, cx| cx.count(Project::query()
	.filter(|other| other.team_id.eq(project.team_id))).gt(10),
	message = "A team has at most ten projects")]
pub struct Project {
	pub id: Id<Project>,
	pub team_id: Id<Team>,
	pub name: String,
}
```

Inside the counted query, a column of the row the rule checks, here `project.team_id`, links the two: the count runs for each project the query looks at, inside the same SQL statement, on Postgres and SQLite alike. `cx.exists(query)` is short for `cx.count(query).gt(0)`, and a count compares with `eq`, `ne`, `gt`, `gte`, `lt`, and `lte`.

A count sees what the request may see, just as `query.count(&db)` would:

- **The counted table's rules apply.** Counting members only finds the memberships this request may read. Soft-deleted rows are left out too, unless the query asks for them with `with_deleted()`.
- **A table's own read rule doesn't apply inside itself.** "Members see the members of their teams" counts members while deciding which members are visible; applying that rule again would never end, so inside it the count reads every member. The same goes for two tables whose read rules count each other.
- **Writes count the row being written.** The cap above checks the stored result, so the new project is part of its own count: the eleventh insert is the one refused.
- **The comparison is yours.** A rule that allows when a count is zero, such as "no one has blocked you", allows for a request without the global it reads, unless you wrap it in `cx.global`.

When the query's table is the rule's own, as in the cap, its columns on the right of a comparison are the checked row's: `other.team_id.eq(project.team_id)` compares each counted project with the one being inserted.

### Enforce policies in the database too

Policies filter every query the database layer writes, but not SQL you write yourself, and not a query someone forgets to run through a request's handle. On Postgres, the same rules can also run as row-level security, so the database itself filters every statement, raw SQL included.

The database can't run Rust closures, so rules read the request through settings: globals whose value Postgres sees as `leptoskit.<name>`.

```rust
use leptoskit::db::Setting;

/// The signed-in user.
pub struct Viewer(pub Id<User>);

impl Setting for Viewer {
	const NAME: &'static str = "viewer";
	type Value = Id<User>;

	fn value(&self) -> Id<User> {
		self.0
	}
}

leptoskit::db::register_setting!(Viewer);

#[table(name = "posts", public)]
#[db::policy(select, |post, cx| post.published.eq(true) | post.author_id.eq(cx.setting::<Viewer>()))]
#[db::policy(insert, update, delete, |post, cx| post.author_id.eq(cx.setting::<Viewer>()))]
pub struct Post { … }
```

`cx.setting::<Viewer>()` works whether or not row-level security is on: put `Viewer` in the request's context with `Context::new().with(Viewer(id))`, as any global. A request without it compares with `NULL`, so the rule matches no row, even negated. `leptoskit_auth`'s `SignedIn` is a setting too, named `account`, holding the account's ID.

Then turn row-level security on, and connect with it:

```toml
[db]
policies = "rls"
```

```rust
let db = Db::connect(&url).await?.with_row_security()?;
```

The next migration creates a policy for each table with rules, for reads, inserts, updates, and deletes, and forces it on the table's owner too, which apps usually connect as. Every statement then runs in a transaction that first sets each registered setting from the request's context, and `db.system(…)` sets a bypass instead; migrations bypass it as well. Results are the same as without it: the database layer still adds the rules to its queries, and a refused write is still `DbError::Forbidden`. A refused insert names the rule's message as before; a save or upsert that Postgres refuses first carries Postgres's message instead.

- **Exporting fails** when a rule can't run in the database: it reads a global with `cx.global` or `cx.get`, reads a setting nobody registered, or counts rows of its own table, which Postgres would check against the same policy forever. The error names the table.
- **Views** read their tables as whoever reads them (Postgres's `security_invoker`), so the tables' policies filter a view too, raw SQL included, whoever owns it. Under row-level security a view therefore shows a request only the rows it may read in those tables, where without it a view's query reads every row. A materialized view still holds every row: migrations and `refresh` bypass the policies to fill it, whoever refreshes it. Postgres can't filter one as it's read, so its own `select` rules apply to the database layer's queries alone, and raw SQL reads all of it.
- **The audit log** opens to raw SQL only for a request whose context holds an `AuditReader`, since the reader's rule is Rust code: Postgres sees that the request holds one, not what it matches. So raw SQL from such a request reads every entry, while the database layer reads what the rule matches. Requests add entries as they change audited tables, and only `db.system(…)` changes or removes one.
- **What it guards against:** a query that skips the request's handle, and raw SQL. Anyone who can run arbitrary SQL can also change the settings, so it isn't a defence against SQL injection, and Postgres superusers bypass it entirely.
- **Soft deletes are updates to Postgres.** On a table with a `deleted_at` column, the update policy also lets a request change a row its `delete` rules allow, as long as the row it leaves is soft-deleted, since that is how the database layer deletes it. Raw SQL from such a request can change other columns in the same statement.
- **Postgres only.** On SQLite, `policies = "rls"` refuses to export and `with_row_security` fails.
- Each statement outside a transaction becomes a short transaction, so it costs a few more round trips. Postgres also checks a row a write returns against the `select` policy, so a rule that lets a request insert a row it may not read is refused.

## Run your own code when rows change

Some rules aren't conditions on a row but work to do whenever it's written: keep an order's total in step with its lines, or record every change of status as an event. Doing that in each server function means one forgotten call leaves the data wrong. Name the hooks on the table and implement them once:

```rust
#[table(name = "orders", hooks(before_write, after_update))]
pub struct Order {
	pub id: Id<Order>,
	pub status: Status,
	pub quantity: i64,
	pub unit_price: i64,
	pub total: i64,
}

impl db::BeforeWrite for Order {
	fn before_write(&mut self, cx: &db::WriteContext<'_, Self>) -> db::Result<()> {
		if cx.old().is_some_and(|old| old.status == Status::Shipped) && self.status != Status::Shipped {
			return Err(DbError::Forbidden { table: "orders".into(), message: "Shipped orders stay shipped".into() });
		}

		self.total = self.quantity * self.unit_price;
		Ok(())
	}
}

impl db::AfterUpdate for Order {
	async fn after_update(old: &Self, new: &Self, tx: &impl db::Connection) -> db::Result<()> {
		if old.status != new.status {
			NewOrderEvent { order_id: new.id, status: new.status }.insert(tx).await?;
		}

		Ok(())
	}
}
```

There are four hooks:

| Hook           | Runs                                                                                 | Gets                                                       |
| -------------- | ------------------------------------------------------------------------------------ | ---------------------------------------------------------- |
| `before_write` | before every insert and update: `insert`, `upsert`, `save`, adding a link            | the row to change, whether it's an insert, and the old row |
| `after_insert` | after an insert                                                                      | the row as stored                                          |
| `after_update` | after an update                                                                      | the old row and the new one, both as stored                |
| `after_delete` | after a delete, soft or for good, including `delete`, `destroy`, and removing a link | the row as the delete left it                              |

Every hook runs inside the write's transaction, and its own writes through `tx` join it, so a hook that fails rolls the whole write back. The row is validated after `before_write`, so a hook can fill in values that the rules then check. A hook's writes obey the same policies as the write that ran it.

Bulk writes, `Order::update()…run(&db)` and `Order::query().delete(&db)`, change rows in one statement without loading them, so they can't run hooks. Rather than skip them quietly, they fail with `DbError::HooksSkipped` on a table that has hooks they'd skip. Write the rows one at a time, or say you mean it with `.allow_skipping_hooks()`.

Two kinds of change never run hooks: rows the database removes itself through `on_delete = cascade`, and rows a soft delete reaches through `on_soft_delete = cascade`. A table with an `after_delete` hook can't be the target of a soft-delete cascade; delete the linked rows in the other table's hook instead.

### Record who wrote a row

`#[db(created_by)]` and `#[db(updated_by)]` fill in who inserted a row and who last wrote it, as `created_at` and `updated_at` fill in when:

```rust
#[table(name = "documents")]
pub struct Document {
	pub id: Id<Document>,
	pub title: String,
	#[db(created_by)]
	pub owner_id: Id<User>,
	#[db(updated_by)]
	pub editor_id: Option<Id<User>>,
}
```

The value is the request's `Actor`. Sign-in adds one to every signed-in request's context; without it, add `Actor::of(user.id)` to the `Context` yourself. A column can be an `Id<T>` or a `Uuid`, optional or not. An optional one is `None` when nobody is acting, such as in a nightly job; a required one refuses the write with `DbError::NoActor`. `db.system("…")` made from a request's handle still records that request's actor.

Neither column is part of `NewDocument`, a `before_write` hook can read them but not change them, and a bulk update can't set them, so they always say who really wrote the row.

### Slugs

`#[db(slug(from = title), unique)]` makes a URL slug from another column when a row is inserted, and keeps it unique by adding `-2`, `-3`, and so on:

```rust
#[table(name = "articles", public)]
pub struct Article {
	pub id: Id<Article>,
	pub title: String,
	#[db(slug(from = title), unique(except = deleted_at))]
	pub slug: Slug,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

let first = NewArticle { title: "Hello, World!".into() }.insert(&db).await?;  // hello-world
let second = NewArticle { title: "Hello world".into() }.insert(&db).await?;   // hello-world-2
```

The slug keeps its ASCII letters and digits, lowercased, with a hyphen for everything else, and is cut to 80 characters; text without either becomes `untitled`. It's set once: saving the row, or upserting over it, keeps it, so links to it keep working. Make a new one in a `before_write` hook with `Slug::from_text` if you need to. `unique(except = deleted_at)` frees a soft-deleted row's slug. Two requests inserting the same title at the same moment can pick the same slug, and the second fails with `DbError::Conflict`.

## Keep an audit log

When something changes that shouldn't have, the first question is who did it, and what was there before. Mark the table `audit`:

```rust
#[table(name = "invoices", audit)]
pub struct Invoice {
	pub id: Id<Invoice>,
	pub amount: i64,
	#[db(secret)]
	pub payment_token: String,
}
```

Every insert, update, and delete the database layer makes to the table, one row at a time or in bulk, and every row a soft-delete cascade reaches, adds an `AuditEntry` in the same transaction: so a write that fails or rolls back records nothing. Each entry holds:

- `op`: `Insert`, `Update`, or `Delete`;
- `table_name` and `row_key`, the row's key as a JSON array;
- `before` and `after`, the row as JSON, without its `secret` columns: an insert has no `before`, and a delete for good no `after`;
- `actor`, who was acting, and `reason`, the reason given to `db.system("…")` for a change that bypassed policies;
- `at`, when.

The first audited table adds a `_leptoskit_audit` table to your next migration.

Reading the log is refused unless the request may: a request reads no entry until its context holds an `AuditReader`, and nothing but a `system` handle can change or remove one.

```rust
// Administrators read everything.
let cx = cx.with(AuditReader::everything());

// Everyone else reads what they did themselves.
let cx = cx.with(AuditReader::matching(|entry, cx| match cx.actor() {
	Some(actor) => entry.actor.eq(actor.uuid()),
	None => Expr::never(),
}));

let history = AuditEntry::for_row(&invoice)?.all(&db).await?;
let purged = AuditEntry::query().filter(|entry| entry.at.lt(cutoff)).destroy(&db.system("retention")).await?;
```

The log also records the rows the database changes on its own when a row is deleted for good: rows it removes through `on_delete = cascade`, as deletes, and links it empties through `set_null`, as updates. Before such a delete, the database layer reads and locks them, so the record matches what the delete did. Raw SQL isn't recorded.

## Views

Some reads are the same aggregate everywhere: each author's post count, a leaderboard. Declare it once as a view, which the migrations create, and read it like a table:

```rust
/// How much each author has written.
#[db::view(name = "author_stats", public, sql = "
	SELECT author_id, count(*) AS posts, max(score) AS best
	FROM posts WHERE deleted_at IS NULL GROUP BY author_id")]
#[db::policy(select, |stats, cx| cx.global(|viewer: &Viewer| stats.author_id.eq(viewer.0)))]
pub struct AuthorStats {
	pub author_id: Id<User>,
	pub posts: i64,
	#[db(private)]
	pub best: i64,
}

let busy = AuthorStats::query().filter(|stats| stats.posts.gt(10)).all(&db).await?;
let author = busy[0].author(&db).await?;
```

The struct's fields are the query's columns, in order; a field takes only `column`, `private`, `secret`, and `in(...)`, and `public` and `projection(...)` give it client types as on a table. When the databases need different SQL, write `sql(postgres = "…", sqlite = "…")`.

A view's query reads every row of its tables, whatever their policies (unless [the database enforces policies too](#enforce-policies-in-the-database-too)), so give the view its own `#[db::policy(select, …)]` rules: they apply to it as to a table, and a view with none is open to server code, as a table with none is. Nothing writes to a view; a bulk write on one fails with `DbError::ReadOnly`.

`materialized` keeps the rows the query returned until you refresh it, for queries too slow to run on every read:

```rust
#[db::view(name = "leaderboard", materialized, sql = "SELECT author_id, count(*) FROM posts GROUP BY author_id")]
pub struct Leaderboard {
	pub author_id: Id<User>,
	pub posts: i64,
}

Leaderboard::refresh(&db).await?;
```

On Postgres it's a materialized view. SQLite has none, so it's a table that `refresh` empties and fills again in one transaction. Either way, readers see the old rows or the new ones, never a mix.

A materialized view takes `index(...)` and `unique(...)` over its columns, as a table does. On Postgres, `refresh` locks out readers while the query runs; with a `unique(...)` index the view also gets `refresh_concurrently`, which lets them carry on and changes only the rows that differ. On SQLite, `refresh_concurrently` is `refresh`, whose transaction already lets readers carry on.

```rust
#[db::view(name = "leaderboard", materialized, unique(author_id), index(posts), sql = "…")]
pub struct Leaderboard { … }

Leaderboard::refresh_concurrently(&db).await?;
```

A view can read another view: migrations create each view after the views its query names, and drop it before them. Shapes can start from a view, too: `AuthorStats::query().shape::<StatsCard>()` with `#[db(from = AuthorStats)]`, following the view's `Id<T>` columns with `via`.

Migrations create views after every table change and drop them before. A view's SQL isn't parsed, so when a migration changes a table in a way a view could notice, it drops and recreates every view, which refills materialized ones.

## Search text

A search box that matches with `ilike '%rust%'` misses `Rust's`, can't rank the best match first, and reads every row. Mark the text columns to search, and the table gets a full-text index that the migrations create and every write keeps current:

```rust
#[table(name = "articles", public, search(language = "english"))]
#[db::policy(select, |article, cx| article.published.eq(true)
	| cx.global(|viewer: &Viewer| article.author_id.eq(viewer.0)))]
pub struct Article {
	pub id: Id<Article>,
	pub author_id: Id<User>,
	#[db(search(weight = A))]
	pub title: String,
	#[db(search(weight = C))]
	pub body: Option<String>,
	pub published: bool,
}

let hits = Article::query()
	.search("passkeys rust")
	.limit(10)
	.all(&db)
	.await?;
```

`search` returns the matching rows, best first. A match in a column with a higher weight counts more: `A` most, then `B`, `C`, and `D`, which is what `#[db(search)]` alone gives. The text reads like a web search box:

| You type          | It finds rows that                      |
| ----------------- | --------------------------------------- |
| `passkeys rust`   | contain both words                      |
| `"memory safety"` | contain the phrase, words in that order |
| `rust -java`      | contain `rust` and not `java`           |
| `rust OR go`      | contain either                          |
| `concur*`         | contain a word starting with `concur`   |

Punctuation and stray quotes never make the search fail; a search with nothing to match, such as `-java` alone, finds nothing. A search has at most 32 words and phrases.

The search is a query like any other: the table's `select` policy decides which rows it can find, soft-deleted rows stay hidden unless you add `with_deleted()`, and `filter`, `order_by` (to break ties between equally good matches), `limit`, `offset`, `first`, and `count` work as they do on `query()`.

To show why a row matched, ask for highlights. `hits` returns each row with its rank (higher is better) and the highlights, in the order you asked:

```rust
let hits = Article::query()
	.search("passkeys")
	.highlight(|article| article.title)
	.excerpt(|article| article.body, 20)
	.hits(&db)
	.await?;

for segment in hits[0].highlights[0].segments() {
	// `segment.text`, and `segment.matched` for the words that matched.
}
```

`highlight` returns a column's whole text and `excerpt` about that many words (2 to 64) around the best matches. A highlight is plain text in runs, never HTML, so render each run escaped, with matched runs wrapped in `<mark>` or styled as you like. Matches are marked on their way from the database with two private-use characters, U+E000 and U+E001, so highlighted text leaves those two out.

**Languages.** `search(language = …)` decides how text is split into words. `simple`, the default, lowercases words and keeps each one as written, so `apple` doesn't find `apples`. `english` reduces words to their stem, so `passkey` finds `passkeys`. On Postgres, any text search configuration the server has works, such as `german`; SQLite has only `simple` and `english`, and the migration is refused for others.

**On each database.** Postgres keeps a generated `tsvector` column, `lk_search`, with a GIN index, and ranks with `ts_rank`. SQLite keeps a copy of the searched text in an FTS5 table, `<table>_lk_search`, numbered by a second table that maps each entry to its row's key, with triggers that update both on every write; it ranks with `bm25`, weighted like Postgres. Both follow every write, including raw SQL. They split words slightly differently: Postgres's `english` leaves out stopwords such as `the` and `again`, which SQLite's keeps, and excerpts may choose different words. Ranks order rows the same way on both, but their values differ.

**Migrations.** Adding `search` to a table indexes the rows it already has. Changing which columns are searched, their weights, or the language, renaming the table, or renaming or retyping a searched or key column recreates the index; it holds only words derived from the table, so nothing is lost. Searched columns must hold text (`String`, `Option<String>`, or a text scalar such as `Email`) and can't be generated.

## Places and shapes

"Shops within 500 metres", "which delivery zone covers this address", and "everything on the visible map" are slow and wrong when written over a pair of latitude and longitude columns: degrees aren't metres, the distance has to be computed for every row, and nothing indexes it. Turn on the `db-geo` feature and store shapes instead:

```toml
[dependencies]
leptoskit = { version = "…", features = ["db-geo"] }
```

```rust
use leptoskit::db::geo::{Geo, Geography, Meters, MultiPolygon, Point, Polygon, Srid, point};

#[table(name = "shops", public)]
pub struct Shop {
	pub id: Id<Shop>,
	pub name: String,
	#[db(geo(index))]
	pub location: Geography<Point>,
	#[db(geo(index, valid))]
	pub delivery_area: Option<Geography<MultiPolygon>>,
	#[db(private)]
	pub site_plan: Option<Geo<Polygon, Srid<27700>>>, // British National Grid, in metres
}

let here = Geography(point!(x: -0.1276, y: 51.5072));

// Within 500 metres, nearest first, with the distance.
let nearby: Vec<(Shop, Meters)> = Shop::query()
	.filter(|shop| shop.location.within_distance(&here, Meters(500.0)))
	.nearest(|shop| shop.location, &here)
	.with_distance()
	.limit(20)
	.all(&db)
	.await?;

// Which shops deliver here?
let delivering = Shop::query()
	.filter(|shop| shop.delivery_area.contains(&here))
	.all(&db)
	.await?;
```

This runs on the server, against Postgres or SQLite; the same code works on both. A client receives `PublicShop` with each shape as GeoJSON, `{"type":"Point","coordinates":[-0.1276,51.5072]}`, which map libraries such as MapLibre and Leaflet draw directly.

**Two kinds of shape.** `Geography<T>` holds longitude and latitude on the earth: distances, areas, and lengths are in metres and follow its curve. Use it for anything on the real globe. `Geo<T, Srid<N>>` holds planar coordinates in the coordinate system with EPSG code `N`, measured in that system's units: national grids, floor plans, game maps. A value in one system can't be compared with a column in another; the code doesn't compile. `T` is any shape from the `geo-types` crate, re-exported in `leptoskit::db::geo`: `Point`, `LineString`, `Polygon`, `MultiPoint`, `MultiLineString`, `MultiPolygon`, `GeometryCollection`, `Rect` (stored as a polygon), or `Geometry` for any of them. The database holds each column to its shape and coordinate system.

**Column options.**

- `#[db(geo(index))]` adds a spatial index, which spatial filters and `nearest` use.
- `#[db(geo(valid))]` refuses shapes that aren't valid, such as a polygon whose edges cross: `validate()` checks it before a write, and the database checks it too.
- Coordinates are checked everywhere: a `Geography` longitude outside −180 to 180, a latitude outside −90 to 90, or any coordinate that isn't a finite number fails to deserialize and fails to be written. With `geo(…)`, `validate()` reports it as a `Coordinates` rule, which a form can show beside the field.

Precise locations are often personal data. Mark a column `private` unless every client may see it.

**Filters.** Each takes a shape in the column's coordinate system and works like any other filter: combine them with `&`, `|`, and `!`, and the table's policies apply.

| Filter                                  | Matches rows whose shape                                     |
| --------------------------------------- | ------------------------------------------------------------ |
| `intersects(&shape)`                    | shares at least one point with it                            |
| `contains(&shape)`                      | holds every point of it                                      |
| `within(&shape)`                        | lies entirely inside it                                      |
| `disjoint(&shape)`                      | shares no point with it                                      |
| `touches(&shape)`                       | meets it only at their boundaries                            |
| `within_distance(&shape, distance)`     | is at most that far away: `Meters` on the globe              |
| `distance_to(&shape).lt(x)`, `.gte(x)`… | is nearer or farther than `x`                                |
| `area().gt(x)`, `length().lte(x)`…      | is larger or smaller (square metres and metres on the globe) |

On the globe, `contains` counts a point on the boundary as inside and `within` does too; for planar shapes a boundary point isn't contained, as in PostGIS. Distances on the globe are measured between points, so `within_distance`, `distance_to`, and `nearest` on a `Geography` column take a `Geography<Point>` column and point; measure other shapes in a projected `Geo` system instead.

**Nearest first.** `query().nearest(|row| row.column, &shape)` returns rows nearest first, with ties broken by the query's own orderings and then the key, and leaves out rows without a shape. `with_distance()` returns each row's distance as well. Give it a `limit`: that's what lets the index stop early.

**On the client.** The shape types are client-safe and serialize as GeoJSON (RFC 7946), and their JSON Schema describes it. For big polygons, `Wkb<Geography<MultiPolygon>>` stores the same column but sends it as base64 well-known binary. The `geo` crate's algorithms are re-exported as `leptoskit::db::geo::algorithms`, so a map component can measure or test shapes in the browser without a round trip; `bounding_box()` gives a shape's extent.

**On each database.**

- **Postgres** uses PostGIS. The first migration with a shape runs `CREATE EXTENSION IF NOT EXISTS postgis`, columns become `geography(Point, 4326)` or `geometry(Polygon, 27700)`, and `geo(index)` creates a GiST index. The database must have PostGIS available: devenv's `services.postgres` takes `extensions = extensions: [ extensions.postgis ];`, the `postgis/postgis` container image has it, and so do most managed Postgres services. The embedded development server doesn't, so set `DATABASE_URL` to one that does. Planar `nearest` uses the index; on the globe it orders by the exact distance on the spheroid, which the index can't, so pair it with `within_distance` on large tables. PostGIS coerces geography coordinates outside the valid range into it rather than refusing them; leptoskit refuses them before they reach it.
- **SQLite** needs nothing installed, on servers and in the on-device database alike. A shape is stored as a GeoPackage geometry, a `BLOB` that the database checks for its shape, coordinate system, and range. `geo(index)` keeps an R*Tree, `rtree_<table>_<column>`, of each row's bounding box, and a table numbering the rows by key, with triggers that follow every write, raw SQL included. Spatial filters narrow candidates by box first, then test each exactly. Distances, areas, and lengths on the globe use the same geodesic algorithm as PostGIS, so they agree to well under a millimetre.

**Known differences.** Both databases give the same answers for valid shapes away from the cases below, and the same tests run on both.

- On the globe, PostGIS draws polygon and line edges as great circles; SQLite's layer draws them straight in longitude and latitude. They agree away from long edges, but a point within metres of an edge many kilometres long can fall on different sides, and SQLite doesn't handle shapes that cross the antimeridian or circle a pole. Keep edges short, or use a projected `Geo` system.
- The two engines (GEOS in PostGIS, the `geo` crate in SQLite) can disagree about degenerate shapes and points exactly on a boundary.

**PostGIS-only operations.** On Postgres, a column can be turned into another shape before it's tested or measured: `buffer(distance)`, `transform::<Srid<3857>>()`, and, on planar shapes, `term().simplify(tolerance)` and `term().union(&shape)`:

```rust
let near_the_river = Plot::query()
	.filter(|plot| plot.boundary.buffer(50.0).intersects(&river))
	.all(&db)
	.await?;
```

They exist in SQLite builds when both drivers are compiled in, but a query using one fails there with `DbError::Encode`. Raster, topology, and routing stay in raw SQL.

**Migrations.** Adding `geo(index)` indexes the rows already there. Renaming the table, renaming or retyping the column, or changing the key recreates the index, which holds nothing that can't be recomputed. Changing a column to `Geometry` (any shape) in the same coordinate system keeps every value; other changes of shape or system need `--allow-destructive`.

**GeoPackage files.** A SQLite database can also be a GeoPackage, the file format GIS tools open as layers. Set it in `leptoskit.toml`:

```toml
[db.geo]
geopackage = true
```

Migrations then keep GeoPackage's metadata tables, registering each table's geometry column, and QGIS and GDAL open the file and read the shapes. GeoPackage allows one geometry column per table, so a table with two fails the migration with a message saying which. GIS tools read the layers without the spatial index, which keys rows differently from GeoPackage's own.

## Refetch what changed

A page that lists posts shows stale data the moment someone else writes one, and polling every few seconds is wasteful and still late. Mark the table `live`, and every committed write to it says which rows changed, so each client refetches exactly the queries those rows touch:

```rust
#[table(name = "posts", public, live)]
#[db::policy(select, |post, cx| post.published.eq(true) | cx.global(|viewer: &Viewer| post.author_id.eq(viewer.0)))]
pub struct Post { … }
```

On the server, give each signed-in client a feed of changes made from its request's handle, and send each one over whatever your app uses to push to clients, such as server-sent events or a WebSocket:

```rust
let mut changes = request_db.changes().await?;
loop {
	let invalidation = changes.next().await?;
	send(serde_json::to_string(&invalidation)?).await?;
}
```

On the client, in the web app or the native app's interface, tag queries with the tables and rows they read, and apply each invalidation with `leptoskit_query` (with the `db` feature, which `leptoskit`'s `db` and `query` features turn on together):

```rust
use leptoskit_query::live::{row_tag, table_tag};

fn posts() -> QueryScope<(), Vec<PublicPost>, ServerFnError> {
	QueryScope::new(|()| list_posts()).with_tags_for(|()| vec![table_tag("posts")])
}

fn post() -> QueryScope<Id<Post>, PublicPost, ServerFnError> {
	QueryScope::new(get_post).with_tags_for(|id| vec![row_tag("posts", [id])])
}

client.apply_invalidation(&invalidation);
```

A change to a post refetches queries tagged with its table and with its own row, and nothing else.

- **Invalidate, don't replicate.** An invalidation carries the table and the changed rows' keys, never their data. The refetch goes through the same policies as any query.
- **Nobody learns about rows they can't see.** A subscriber hears of a change only when its `select` policy allows the row as it was before the write or as it is after it, so a post made private still reaches the readers who saw it, and a private draft reaches only its author. Secret columns never take part, so a policy that reads one never matches a change: it fails closed.
- **Only what commits.** Changes are published when their transaction commits, all of one transaction's together, and never for one that rolls back. Rows the database deletes or empties through `on_delete` cascades are published too.
- **Every server hears every change.** On Postgres, changes travel through `LISTEN`/`NOTIFY`, so a write on one server reaches subscribers on all of them; the first `changes()` call opens a listening connection. On SQLite they stay in the process.
- **When changes may be missed**, a feed returns `Invalidation::Everything`: a subscriber that fell far behind, a server that lost its listening connection and reconnected, or a change to a table this build doesn't know, as during a rolling deploy. `apply_invalidation` then refetches every query with a database tag.
- **What isn't published:** raw SQL, migrations, and refreshing a materialized view.

Each write to a `live` table reads the rows it changes, as an audited table's do, and runs in a transaction.

## Tables that come from a library

Some features bring their own tables: sign-in needs accounts and sessions, and an audit log needs its entries. A library ships them as a **schema pack**, and your app adds it in the crate that declares your tables:

```rust
leptoskit::db::packs!(leptoskit_auth);
```

The pack's tables join your schema, in the pack's own namespace (`auth.sessions` on Postgres, `auth_sessions` on SQLite). They show up in your next `leptoskit db migration create` for you to review, like changes to your own tables. Your code reads them, and links to them with `Id<auth::Account>`, under the pack's policies. Only the pack's own functions write them: their generated write methods are private to the pack's crate.

To write a pack, give its tables a `pack`:

```rust
#[db::table(name = "sessions", pack = "auth")]
pub struct Session { … }
```

A pack serves apps on either database, so its tables must work on both. Exporting the schema refuses pack tables with a type only one database has, a raw SQL default or check, or a link to a table outside the pack.

## When two accounts merge

With [sign-in](23-sign-in.md), people can merge a second account into their main one and bring its data along, as described in [merging accounts](auth/merging.md). Your tables say how their rows move, so most apps write no merge code.

The schema already knows which tables hold an account's data: every foreign key to `auth.accounts`, or to a table that shares the account's key, such as your `users`. Each table's rows move by its `on_account_merge` strategy:

```rust
#[table(name = "posts")]                                                   // reassign, the default
pub struct Post { /* author_id: Id<Account> */ }

#[table(name = "follows", unique(follower_id, topic), on_account_merge = reassign_skip_conflicts)]
pub struct Follow { /* follower_id: Id<Account>, topic: String */ }

#[table(name = "prefs", on_account_merge = keep_newest)]                   // one row each, with updated_at
pub struct Pref { /* #[db(unique)] user_id: Id<Account>, … */ }

#[table(name = "drafts", on_account_merge = keep_survivor)]
pub struct Draft { /* user_id: Id<Account> */ }

#[table(name = "balances", on_account_merge = custom)]                     // your OnMerge moves them
pub struct Balance { /* #[db(unique)] user_id: Id<Account>, cents: i64 */ }
```

| Strategy                  | What happens to the other account's rows                                                  |
| ------------------------- | ----------------------------------------------------------------------------------------- |
| `reassign` (the default)  | They move. One that breaks a unique constraint stops the whole merge, and nothing changes |
| `reassign_skip_conflicts` | They move, except those that would duplicate one of the survivor's, which are dropped     |
| `keep_newest`             | For one row per account: whichever of the two rows was updated more recently stays        |
| `keep_survivor`           | They're dropped; the survivor's rows are the ones that count                              |
| `custom`                  | Your `auth::OnMerge::merge_custom` moves them, in the merge's transaction                 |

A table keyed by the account, such as `users`, keeps the survivor's row; `OnMerge::merge_profile` can copy anything worth keeping from the other one before it's deleted. Its strategy may only be `keep_survivor` (the default) or `custom`.

A reference inside a unique constraint, like `follows` above, would collide if moved blindly, so `leptoskit db check` fails until its table says what to do:

```text
merging accounts can't bring their data along until every table says how:
  follows.follower_id is in a unique constraint, so merging accounts could collide: add `on_account_merge = reassign_skip_conflicts`, `keep_survivor`, `keep_newest`, or `custom` to #[db::table]
```

Until every table is covered, and while any table is `custom` without your `OnMerge`, people can still merge accounts, but only by moving the sign-in methods and deleting the other account's data. A strategy changes no tables, so it never needs a migration. Foreign keys only count when they're declared: a column marked `no_foreign_key` isn't found.

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

Run `leptoskit db check` in CI. It fails when the tables changed without a migration. With sign-in's tables, it also fails when a table that holds an account's data can't follow a merge of two accounts; see [when two accounts merge](#when-two-accounts-merge).

### While `leptoskit dev` runs

Stopping the dev loop to write a migration after every change to a table breaks your flow. So `leptoskit dev` watches the crate that declares the tables, and when they change it compares them with the latest migration before it rebuilds the app:

```console
reading the tables in my_app_db
[db] the tables changed since the latest migration:
  + column notes.pinned (bool)
[db] wrote migration 20261030T141502_column_notes_pinned_bool; review it, then commit it with schema.json and registry.txt
[db] applied 20261030T141502_column_notes_pinned_bool to the development database
[web] rebuilt in 9.1s
```

- **Changes that keep every row**, such as a new table, column, or index, become a migration, as `leptoskit db migration create` would write it, applied to the development database straight away. Each change is its own migration; review them before you commit, as you would one you wrote.
- **Changes that would delete data**, such as removing a column, write and apply nothing. `leptoskit dev` lists them and asks you to run `leptoskit db migration create <name> --allow-destructive` yourself, so data is never deleted without you saying so.
- **Changes that need an answer**, such as a column that disappeared while another appeared, write nothing either. Mark a rename with `#[db(renamed_from = "...")]`, or run `leptoskit db migration create` and answer its question.
- **A migration the database refuses**, such as a new check that existing rows break, is removed again, and the error says why.

Set `watch = "report"` under `[db]` to only print what changed, or `watch = "off"` to ignore the tables.

#### Push changes without migrations

While an idea takes shape, the tables change many times, and a migration for each step is noise to review and squash later. Set `watch = "push"` under `[db]`, and `leptoskit dev` applies each change straight to the development database without writing a migration:

```console
[db] pushed to the development database:
  + column notes.pinned (bool)
```

When the tables are right, `leptoskit db migration create <name>` writes one migration from the latest committed one to the tables as they are now, and the development database takes it over without running it, so its rows stay. `leptoskit db push` does what the dev loop does, outside it.

- **Deleting data needs consent.** A push that would drop a column or a table pushes nothing and says so; `leptoskit db push --allow-destructive` pushes it.
- **No migration lands on top of pushes.** While the development database has pushes, the server refuses to apply a migration it hasn't had, such as a teammate's you just pulled, and says to create your migration or `leptoskit db reset --yes`. Migrations it already has are fine, so the server still starts.
- **`leptoskit db status`** says when the database has pushes no migration records yet.

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
| `leptoskit db push`        | Applies the tables to the database without a migration, as push mode does.                                   |
| `leptoskit db branch list` | Lists each branch's database on the development Postgres; `prune --yes` deletes those of finished branches.  |

They read the URL from `--url`, or from `DATABASE_URL`. A Postgres app with neither uses [the development database](#a-database-while-you-develop).

## Configuration

```toml
[db]
dialect = "postgres" # or "sqlite"
crate = "my_app_db" # the crate that declares the tables; this is the default
features = ["server"] # the features that compile its server side
migrations = "migrations" # relative to the project root
url_env = "DATABASE_URL" # where status, migrate, and reset find the database
dev_server = "auto" # "devenv", "embedded", or "off"; Postgres only
watch = "migrate" # or "push", "report", or "off"
branch_databases = true # a database per git branch on the development Postgres
policies = "orm" # or "rls": Postgres enforces policies too

[db.geo]
geopackage = false # SQLite: keep GeoPackage metadata for GIS tools
```

An empty `[db]` table uses every default. `dev_server` picks the development database when `DATABASE_URL` isn't set: `auto` uses devenv's `services.postgres` when `devenv.nix` turns it on and the embedded server otherwise, `devenv` and `embedded` choose one, and `off` starts none. `watch` says what `leptoskit dev` does when the tables change.
