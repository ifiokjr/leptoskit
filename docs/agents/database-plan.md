# Plan: `leptoskit_db`, a built-in database layer

Status: **approved** (revision 2.2). Implementation is in progress; each phase lands as its own pull request.

Revision 2.1 records two decisions:

- MySQL is out of scope entirely;
- dev Postgres works as D6 recommends.

Revision 2.2 adds two features for a later phase (8):

- first-class geospatial support (feature `geo`): PostGIS on Postgres, and an API-compatible layer on SQLite (§4.13);
- vector embeddings on both databases (feature `vector`, §4.14).

Companion plan: [auth-plan.md](auth-plan.md) covers sign-in, MFA, passkeys, third-party login, and acting as an identity provider for other sites. It is built on this database layer.

## What changed since revision 1

| Area            | Revision 1                        | Revision 2                                                                                                                                                                                                                                                                                                                       |
| --------------- | --------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Dialects        | SQLite first, Postgres in phase 5 | **Postgres is the default** and SQLite is a supported feature; nothing else. Each dialect unlocks its own column types and capabilities.                                                                                                                                                                                         |
| IDs             | UUIDv7 `Id<T>`                    | Same, now explicitly on the `uuid` crate. A field named `id: Id<Self>` is the key by convention, with no attribute needed.                                                                                                                                                                                                       |
| Modelling power | Tables, columns, FKs              | Gel's datamodel as annotated Rust: links and backlinks, deletion policies, multi links with link properties, constraints with client validation, custom scalars, mixins, polymorphic tables, computed and generated columns, rewrites, triggers, globals, **access policies**, views, annotations, modules, branches, extensions |
| Queries         | Filters and relations             | Adds **shapes**: Gel-style nested selects into annotated structs, in one round trip                                                                                                                                                                                                                                              |
| Modules         | None                              | **Schema packs**: crates that ship tables (auth, audit, search, vector, storage) and land in the app's own migration history                                                                                                                                                                                                     |

## Goals

1. Rust structs are the schema. Changing a struct produces a reviewed, committed migration; nobody hand-writes DDL.
2. The server sees every column. A client can only ever receive what the type system allows.
3. Gel's expressiveness (links, policies, shapes, mixins, constraints) without a new query language. Everything is attributes on Rust items, checked by the compiler.
4. One declaration feeds the database, the server and the client. A constraint becomes a `CHECK`, a server-side validation, a client-side form validation, and a JSON Schema rule in the generated TypeScript and Dart clients.
5. No third-party sync engine.

## Non-goals

- A new query language. Escape hatches are typed raw SQL, not a DSL parser.
- Other databases. Postgres and SQLite are the only dialects. MySQL in particular is out (see D1): it has no transactional DDL, `RETURNING`, partial indexes or RLS, so every feature would need a fallback.
- Hiding SQL. The generated migrations are plain SQL you can read in review.

## 1. Dialects and features

```toml
# leptoskit facade features
db = ["dep:leptoskit_db"] # client-safe core: Id, Public types, validation
db-postgres = ["db", "leptoskit_db/server", "leptoskit_db/postgres"]
db-sqlite = ["db", "leptoskit_db/server", "leptoskit_db/sqlite"]
db-geo = [
	"db",
	"leptoskit_db/geo",
] # PostGIS on postgres; the compatible layer on sqlite (§4.13)
db-vector = [
	"db",
	"leptoskit_db/vector",
] # pgvector on postgres; sqlite-vec on sqlite (§4.14)
```

`geo` and `vector` are dialect-neutral switches. Combined with `postgres` or `sqlite`, they pull in that database's backend; on their own, in client builds, they only add the wasm-safe types (`geo-types`, GeoJSON serde, `Vector<N>`).

Cargo can't express "Postgres unless something else is chosen", and client builds must pull in no driver at all. So the default lives in the scaffold: `lk new --db` generates `server = ["leptoskit/db-postgres"]` unless you pass `--db sqlite`.

**A project has one dialect**, set in `leptoskit.toml` (`[db] dialect = "postgres"`), and its migrations are SQL for that dialect. Libraries are different: schema packs such as auth must run on every dialect, so they use the **portable subset** and ship tested SQL for each one. An app can opt into portable mode too (decision D2). Its migrations are then written to `migrations/postgres/` and `migrations/sqlite/`, and the exporter rejects any column or feature outside the portable subset.

### What each dialect unlocks

| Capability                                      | Postgres                             | SQLite                                                              |
| ----------------------------------------------- | ------------------------------------ | ------------------------------------------------------------------- |
| `Id<T>` storage                                 | `uuid`                               | `BLOB` (16 bytes)                                                   |
| `Timestamp`                                     | `timestamptz`                        | `TEXT`, fixed-width RFC 3339, UTC, sortable                         |
| `Json<T>`                                       | `jsonb`                              | `TEXT` with `json_valid` check                                      |
| Arrays: `Vec<T>` of scalars                     | native arrays                        | `Json<Vec<T>>` only                                                 |
| `Decimal`                                       | `numeric`                            | `TEXT` (exact) or `REAL` (opt-in)                                   |
| `INSERT … RETURNING`, upsert                    | yes                                  | yes                                                                 |
| Partial unique (`except`)                       | partial index                        | partial index                                                       |
| `CHECK` constraints                             | yes                                  | yes                                                                 |
| `regex` constraint in the database              | `~` operator                         | app-side only (no built-in `REGEXP`)                                |
| Generated columns                               | `STORED`                             | `STORED`/`VIRTUAL`                                                  |
| Transactional DDL (failed migration rolls back) | yes                                  | yes                                                                 |
| Access policies enforced in the database (RLS)  | optional `rls` mode                  | ORM-enforced only                                                   |
| Live changes across instances                   | `LISTEN/NOTIFY`                      | single instance                                                     |
| Full-text `#[db(search)]`                       | `tsvector` + GIN                     | FTS5 table                                                          |
| Vectors `Vector<N>` (feature `vector`)          | `pgvector`: HNSW and IVFFlat indexes | `sqlite-vec`, statically linked; exact search                       |
| Geospatial (feature `geo`)                      | PostGIS: every operation             | Built-in compatible layer: portable operations with an R*Tree index |
| Schemas as namespaces                           | real schemas (`auth.sessions`)       | table prefix (`auth_sessions`)                                      |

**How a type limits itself to dialects.** Column types implement `Column` only under the features that support them; for example, `impl Column for Vector<N>` exists only with `postgres`. In a SQLite-only app, using `Vector<1536>` fails to compile with "`Column` is not implemented for `Vector<1536>`". When features unify and both drivers are present, the exporter checks every column against the project's dialect and names the field that doesn't fit.

## 2. IDs

```rust
pub struct Id<T: ?Sized> { uuid: uuid::Uuid, _table: PhantomData<fn() -> T> }
```

- **New IDs are UUIDv7** (`Uuid::now_v7()`). They're time-ordered, so B-tree indexes stay compact and `ORDER BY id` approximates creation order.
- **Serialized as the hyphenated string**, so they read naturally in JSON and in the generated TS and Dart clients.
- **Deterministic IDs** (your openbudget pattern): `Id::<Wallet>::from_natural_key(("solana", address))` produces a UUIDv5 in a namespace derived from the table name. Use it for idempotent upserts by natural key.
- **The key is found by convention:** a field `id: Id<Self>` is the primary key with no attribute. `#[db(key)]` marks natural or composite keys, and `#[db(key, generate = false)]` covers keys the caller supplies.
- **Conversions:** `Id<T>` is `Copy`, `Hash`, `Ord` and `Display`, and has `.uuid()`, `Id::parse(&str)` and `From<Uuid>` behind an explicit `Id::from_uuid`. That last one is explicit so nobody turns an `Id<Post>` into an `Id<User>` by accident.

## 3. Declaring tables

```rust
use leptoskit::db;
use leptoskit::db::Id;
use leptoskit::db::Timestamp;

/// Someone with an account. Doc comments become COMMENT ON and JSON Schema descriptions.
#[db::table(name = "users", public)]
pub struct User {
	pub id: Id<User>,
	#[db(unique(except = deleted_at))]          // a handle frees up when its owner is soft-deleted
	pub handle: Handle,                         // custom scalar: validated everywhere (§4.4)
	#[db(max_len = 80)]
	pub display_name: String,
	#[db(private)]
	pub email: Email,
	pub role: Role,
	#[db(flatten)]
	pub times: db::Timestamps,                  // mixin: created_at + updated_at (§4.5)
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}
```

Revision 1's rules still apply:

- explicit table names;
- column names are field names;
- `public` tables expose everything except `private` and `secret` fields;
- `Id<T>` columns are foreign keys;
- the server row has no `Serialize`;
- on the client, the row type is an uninhabited marker.

Generated for each table, as in revision 1:

- `PublicUser`: always compiled, with serde and `JsonSchema`, and carrying validation (§4.3);
- the server-only items: the `User` row, `NewUser`, typed columns, `From<User> for PublicUser`, the schema descriptor and relation accessors.

## 4. Gel's datamodel as Rust

Each subsection shows the Gel SDL and then the Rust equivalent. The tier column in the [feature map](#feature-map) says when each one lands.

### 4.1 Links, backlinks and deletion policies

```text
# Gel
type Post {
  required author: User { on target delete delete source; }
}
type User { multi posts := .<author[is Post]; }
```

```rust
#[db::table(name = "posts", public)]
pub struct Post {
	pub id: Id<Post>,
	#[db(on_delete = cascade, backlink = posts)]
	pub author_id: Id<User>,                 // single link = FK column
	pub editor_id: Option<Id<User>>,         // optional link; on_delete defaults to set_null
	pub title: String,
}

// Generated, in both directions:
post.author(&db).await?;                                // -> User
user.posts().order_by(|p| p.created_at.desc()).all(&db).await?;  // backlink from `backlink = posts`
```

Deletion policies map to SQL where SQL can express them:

| Gel                                   | `#[db(on_delete = …)]`                  | SQL                                                                                                    |
| ------------------------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `restrict` (default)                  | `restrict` (default for required links) | `ON DELETE RESTRICT`                                                                                   |
| `delete source`                       | `cascade`                               | `ON DELETE CASCADE`                                                                                    |
| `allow` (optional link becomes empty) | `set_null` (default for `Option`)       | `ON DELETE SET NULL`                                                                                   |
| `deferred restrict`                   | `deferred_restrict`                     | `DEFERRABLE INITIALLY DEFERRED` (Postgres only; SQLite checks FKs at commit with `defer_foreign_keys`) |
| soft delete of the target             | `on_soft_delete = cascade`              | ORM-level: soft-deleting a user soft-deletes their posts in the same transaction                       |

**As built (accessors and backlinks):**

- **Generated methods.** Every `Id<T>` column of the table itself gets two methods:
  - an accessor named after the field without `_id`: `post.author(&db)` returns `T` for a required link, and `Option<T>` for an optional one;
  - a query constructor, `Post::by_author(id)`.
- **Backlinks.** `backlink = posts` adds `user.posts()`, which returns `Post::by_author(user.id())` as an ordinary `Query`. It is an inherent method, so the linked table must be in the same crate; `by_…` covers tables in packs.
- **`HasId`.** Tables keyed by a single `Id<Self>` implement it, and links are typed against it.
- **Limits.**
  - Soft-deleted targets follow the query default: a required link to one is `NotFound`.
  - A mixin's `Id<T>` columns get no accessors, since their types aren't visible to the table's macro.
  - Link names may not collide with each other or with the generated methods. **As built (deletion policies):**

- **Deferred keys.** `on_delete = deferred_restrict` renders a `DEFERRABLE INITIALLY DEFERRED` key with `NO ACTION`, because Postgres can't defer `RESTRICT`. The deferred `NO ACTION` makes the same check at commit, and works on SQLite too.
- **Registering cascades.** `on_soft_delete = cascade` registers a `SoftCascade` through `inventory`. The macro rejects it on a column that isn't a link, or on a table with nothing to soft-delete.
- **Running them.** Single and bulk soft deletes walk the cascades breadth-first with `UPDATE … RETURNING key`:
  - linked rows get the same `deleted_at` and `updated_at`, and their versions move;
  - rows already deleted are skipped, which ends cycles;
  - a bulk delete returns how many rows of its own table it deleted.
- **Atomicity.** The walk runs in one transaction. A new hidden `Connection::begin_unless_in_transaction` opens one on a pool and joins the caller's inside a `Tx`.
- **Versions.** A soft delete now increments the row's own version too.

### 4.2 Multi links and link properties

Gel's `multi` links are join tables. Here you declare the join table as a struct, so link properties are just fields:

```rust
/// A tag on a post. Gel: `multi tags: Tag { added_by: User; position: int16 }`.
#[db::link(name = "post_tags", from = Post as tags, to = Tag as posts, unique)]
pub struct PostTag {
	pub added_by: Id<User>,
	pub position: i16,
	#[db(created_at)]
	pub added_at: Timestamp,
}

// Generated:
post.tags().all(&db).await?;                       // Vec<(Tag, PostTag)>
post.tags().ids(&db).await?;                       // Vec<Id<Tag>>
post.tags().add(tx, tag.id, PostTag { added_by, position: 0, .. }).await?;
post.tags().remove(tx, tag.id).await?;
tag.posts().count(&db).await?;
```

- **No link properties?** Use the table-level shorthand `#[db::table(..., links(tags: Tag))]`. It generates the join struct with no extra fields.
- **`unique` on `#[db::link]`** puts a unique constraint on the `(from, to)` pair.
- **`exclusive`** makes a target linkable from only one source, which gives Gel's one-to-many through a join table.
- **Orphan cleanup** (Gel's `on source delete delete target if orphan`) is `#[db::link(..., on_source_delete = delete_orphan_targets)]`. It runs in the ORM inside the same transaction.

### 4.3 Constraints, enforced in four places

```rust
#[db::table(name = "players", public, check(name = "score_in_range", sql = "score BETWEEN 0 AND 100"))]
#[db::table(unique(game_id, seat))]                              // Gel: constraint exclusive on ((.game, .seat))
pub struct Player {
	pub id: Id<Player>,
	pub game_id: Id<Game>,
	#[db(min = 0, max = 7)]
	pub seat: i16,
	#[db(min_len = 2, max_len = 25, regex = r"^[A-Za-z0-9_]+$")]
	pub nickname: String,
	#[db(one_of("red", "green", "blue"))]
	pub colour: String,                       // prefer a db::Enum; one_of exists for Gel parity
	pub score: i32,
}
```

| Declared once…                     | Database (`CHECK`, `UNIQUE`) | Server (`insert`/`save`)                                | Client (`PublicPlayer::validate`, forms)         | Generated clients (JSON Schema) |
| ---------------------------------- | ---------------------------- | ------------------------------------------------------- | ------------------------------------------------ | ------------------------------- |
| `min`, `max`, `min_len`, `max_len` | yes                          | yes, before SQL runs                                    | yes                                              | `minimum`, `maxLength`, …       |
| `regex`                            | Postgres; app-side on SQLite | yes                                                     | yes (the `regex` crate in wasm; opt-in for size) | `pattern`                       |
| `one_of`                           | yes                          | yes                                                     | yes                                              | `enum`                          |
| `unique`, `unique(except = …)`     | (partial) unique index       | maps the violation to `DbError::Conflict { field }`     | no; needs the server                             | none                            |
| `check(sql = …)`                   | yes                          | checked by the database; the error names the constraint | no                                               | none                            |

- **Validation errors are structured:** `ValidationErrors { field: "nickname", rule: MaxLen(25) }`. The same type comes back from server functions, so a form can show a server-side `Conflict` on the right field.
- **Generated validators run where you need them:** `NewPlayer::validate()` on the server and `PublicPlayer::validate_field("nickname", value)` on the client.

### 4.4 Custom scalars

```text
# Gel
scalar type username extending str { constraint regexp(r'^[A-Za-z0-9_]{4,20}$'); }
```

```rust
/// A public handle. Parse, don't validate: a Handle that exists is valid.
#[derive(db::Scalar, Clone, Debug, PartialEq, Eq, Hash)]
#[db(min_len = 4, max_len = 20, regex = r"^[A-Za-z0-9_]+$", normalize = lowercase)]
pub struct Handle(String);

let handle = Handle::parse("Ifiok_Jr")?;   // -> "ifiok_jr"; Err(ValidationError) otherwise
```

A `db::Scalar`:

- implements `Column` using its inner type's storage, with its own `CHECK` constraints;
- implements `Deserialize` through `parse`, so invalid values from a client never reach a server function;
- emits its rules into JSON Schema.

Built-in scalars:

- `Email` (normalized, with a pragmatic validity check rather than full RFC 5322);
- `Url` and `Slug`;
- with the `solana` feature, `Pubkey` and `Signature`.

**As built:** `Email`, `Url` and `Slug` shipped with the derive. A scalar wraps one `String` or number, so the rest moved: `NonEmpty<String>` is `min_len = 1` on a text scalar; `Money` has two columns, so it is a mixin (§4.5); and `Pubkey` and `Signature` land with Solana wallets in phase Auth 7. `parse` takes `impl Into<String>` for text and the exact type for numbers, so integer literals infer.

### 4.5 Mixins and abstract types

Gel's `abstract type HasTimestamps` plus `type User extending HasTimestamps` becomes a flattened mixin struct, which is explicit, readable, and has serde-style precedent:

```rust
/// A reusable set of columns. Mixins have no table of their own.
#[db::mixin]
pub struct Audited {
	#[db(created_by)]                       // filled from the request's current user (§4.9)
	pub created_by: Option<Id<User>>,
	#[db(updated_by)]
	pub updated_by: Option<Id<User>>,
}

#[db::table(name = "documents", public)]
pub struct Document {
	pub id: Id<Document>,
	pub title: String,
	#[db(flatten)]
	pub times: db::Timestamps,              // built-in mixin: created_at, updated_at
	#[db(flatten)]
	pub audit: Audited,
}

doc.times.updated_at;                       // fields live where you'd expect
```

What a mixin carries:

- **Columns:** flattened ones keep their own names. Use `#[db(flatten, prefix = "billing_")]` to embed the same mixin twice, for example a billing and a shipping `Address`.
- **Rules:** a mixin's indexes, constraints and policies are inherited.
- **Built-in mixins:** `db::Timestamps`, `db::SoftDelete` and `db::Versioned`. `db::Versioned` adds a `version: i64` column that `save` checks and increments, giving optimistic concurrency.

**Shared behaviour across tables**: Gel's "query every `HasTimestamps`" becomes an ordinary Rust trait that the macro implements: `impl db::HasMixin<Audited> for Document`. You can write generic functions over any table with a given mixin:

```rust
async fn recently_touched<T: db::Table + db::HasMixin<db::Timestamps>>(db: &Db) -> db::Result<Vec<T>> {
	T::query().order_by(|t| t.mixin::<db::Timestamps>().updated_at.desc()).limit(20).all(db).await
}
```

**As built:**

- **How a table sees a mixin.** A proc macro can't read another struct's fields, so `#[db::mixin]` also declares a hidden `macro_export` macro under the mixin's own name. `#[db::table]` calls it for each flattened field, and the mixin's macro hands its fields back to `__private::table_continue!`. Once every mixin has answered, the table expands as if the columns were declared in place.
- **Naming types across modules.** Field types never cross into the table's scope. Each field's type is named as `<M as MixinField<N>>::Type`, the public and insert types as `<M as Mixin>::Public` and `<M as Mixin>::New`, and foreign keys are detected at run time through `Column::references`. The table's generated code resolves names at the macro's call site (`Span::resolved_at`), because an expansion reached through a mixin's macro carries `macro_rules` hygiene.
- **Public and insert types.** The public type nests each mixin's `Public…` type rather than flattening it into the JSON, so two prefixed copies can't collide. An insert takes the mixin itself, a generated `New…` struct, or nothing, depending on which fields the ORM fills in.
- **Versioning.** `db::Versioned` uses a new `#[db(version)]` marker (`Table::VERSION`). `save` adds `AND version = <read>` and returns `DbError::Stale` when another write got there first; bulk updates increment the version.
- **Moved out.** `HasMixin` gives generic code the mixin's values. Typed column handles per mixin (`t.mixin::<Timestamps>().updated_at`) are left for later, since `T::UPDATED_AT` covers the common case. `#[db(created_by)]` and `#[db(updated_by)]` need request context, so they land with globals in phase 4.

### 4.6 Polymorphic tables

Gel lets a link target an abstract type and query every subtype at once. Your openbudget `ActorModel` did this by hand with triggers that created supertype rows. In Rust, the natural polymorphic type is an **enum**:

```rust
/// Anything that can own a budget.
#[db::polymorphic(name = "actors", public)]
pub enum Actor {
	User(User),
	Group(Group),
	Organization(Organization),
}

#[db::table(name = "budgets", public)]
pub struct Budget {
	pub id: Id<Budget>,
	pub owner_id: Id<Actor>,                // FK to the supertype table
	pub name: String,
}

match budget.owner(&db).await? {            // exhaustive: adding a variant breaks every match
	Actor::User(user) => …,
	Actor::Group(group) => …,
	Actor::Organization(org) => …,
}
let all = Actor::query().all(&db).await?;   // Vec<Actor>, via one query per variant table, merged by id
```

**Storage is class-table inheritance:**

- `actors(id, kind)` holds one row per subtype row;
- each variant's `id` is also a foreign key to `actors.id`;
- inserting a `User` writes both rows in one transaction, with no database triggers.

`Id<User>` converts into `Id<Actor>` with `.into()`, and `Id<Actor>::downcast::<User>(&db)` goes the other way.

### 4.7 Computed values

Gel computeds are evaluated at query time. There are three Rust equivalents:

| Need                                             | Use                                                                                                                                 |
| ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| Derived in memory only                           | A plain Rust method on the row or on `PublicX`                                                                                      |
| Filterable or indexable, derived from columns    | `#[db(generated = "lower(handle)")] pub handle_key: String`, a stored generated column (read-only; excluded from `NewX` and `save`) |
| Derived from other tables (counts, latest child) | A field on a **shape** (§5.2): `#[db(count(posts))] post_count: i64`                                                                |

`generated` takes dialect SQL. In portable mode, write `generated(postgres = "…", sqlite = "…")`.

### 4.8 Mutation rewrites and triggers

**Gel rewrites** (set a value on the row being written) become field markers for the common cases:

- `created_at`, `updated_at`, `deleted_at`
- `created_by`, `updated_by` (from the current user)
- `version`
- `#[db(slug(from = title))]`, which fills a unique slug on insert

**Custom rewrites** go in a hook trait. Hooks run inside the ORM, in the same transaction:

```rust
impl db::BeforeWrite for Post {
	fn before_write(&mut self, cx: &db::WriteContext<'_, Self>) -> db::Result<()> {
		if cx.is_update() && cx.old().title != self.title {
			self.slug = Slug::from_title(&self.title);
		}
		Ok(())
	}
}
```

**Gel triggers** (side effects after a write) become `AfterInsert`, `AfterUpdate` and `AfterDelete`, with access to `old` and `new`, inside the same transaction:

```rust
impl db::AfterUpdate for Order {
	async fn after_update(old: &Order, new: &Order, tx: &db::Tx) -> db::Result<()> {
		if old.status != new.status {
			NewOrderEvent { order_id: new.id, from: old.status, to: new.status }.insert(tx).await?;
		}
		Ok(())
	}
}
```

- **Audit logs without writing triggers:** `#[db::table(audit)]` records every change in `_leptoskit_audit(table, key, op, actor, before, after, at)`. `private` and `secret` fields are redacted.
- **Bulk writes and hooks.** Bulk `update()` and `delete()` apply the built-in markers in SQL (`SET updated_at = now()`), but they skip custom hooks, because they never load rows. The builder says so: a table with custom hooks requires `.allow_skipping_hooks()` on bulk writes. You can't skip them by accident.

### 4.9 Globals and access policies

This is Gel's most valuable idea. Policies sit on the table, and every query that goes through a request-scoped `Db` obeys them.

```rust
#[db::table(name = "posts", public)]
#[db::policy(select, |p, cx| p.published_at.is_not_null() | p.author_id.eq(cx.user_id()))]
#[db::policy(insert, update, delete, |p, cx| p.author_id.eq(cx.user_id()),
             message = "Only the author can change a post")]
#[db::policy(all, |_, cx| cx.has_role(Role::Admin))]
#[db::policy(deny insert, |p, cx| cx.count(Post::by_author(cx.user_id())).gt(500),
             message = "Post limit reached")]
pub struct Post { … }
```

**Semantics, copied from Gel:**

- A table with no policies is open to server code.
- Once a table has any policy, every operation is denied unless an `allow` matches.
- `deny` overrides `allow`.
- `select`, `update` and `delete` filter silently: rows you may not see simply aren't there.
- A rejected `insert`, or an update whose result isn't allowed, returns `DbError::Forbidden { message }`.

**Globals** are the request context, `cx`:

- `cx.user_id()`, `cx.has_role(...)` and `cx.session()` come from the auth module ([auth-plan.md](auth-plan.md)).
- Apps declare their own globals: `#[db::global] pub struct Tenant(Id<Organization>);`, read with `cx.get::<Tenant>()`.

**Bypassing policies is explicit:**

```rust
let db = Db::from_context()?;               // policies enforced for the signed-in user
let admin = db.system("nightly purge");     // bypass; the reason goes to the audit log and tracing
```

**Enforcement is in two layers** (decision D3):

- **ORM layer (every dialect):** policy closures are the same typed expressions as `filter`. They're added to the SQL as `WHERE` and `CHECK` logic, with `cx` values as bound parameters.
- **Postgres RLS (optional, `[db] policies = "rls"`):** the same expressions are also rendered into `CREATE POLICY` statements in the migration. Each transaction runs `SET LOCAL leptoskit.user_id = …`, so even raw `db.sql(...)` calls are filtered by the database itself.

**Projections per audience** (decision D4) give field-level visibility on top of row policies:

```rust
#[db::table(name = "users", public, projection(SelfUser))]
pub struct User {
	#[db(private, in(SelfUser))]
	pub email: Email,                       // in SelfUser, not in PublicUser
	…
}
// Server function `me()` returns SelfUser; everyone else gets PublicUser.
```

**As built (phase 4, policies and globals):**

- **Globals and `Context`.** Globals are any `Send + Sync` value in a `Context`, found by type, rather than a `#[db::global]` attribute. `cx.global(|v: &Viewer| …)` builds a condition from one and is `Expr::never()` without it.
  - `cx.user_id()` and `cx.has_role(…)` come from `leptoskit_auth`, as globals it defines.
  - `cx.count(…)` subqueries aren't built yet, so rules are column conditions for now.
- **Access.** A `Db` carries an `Access`:
  - `Db::connect` gives an empty `Context`, so policies fail closed;
  - `with_context` scopes a handle to a request;
  - `system(reason)` bypasses policies, keeping the reason for the audit log in phase 5;
  - transactions inherit the access of the `Db` that began them.
- **Reads, updates and deletes** add the action's condition to `WHERE`: `allow`s OR-ed, AND NOT the OR of the `deny`s. A table without an `allow` for an action gets `1 = 0`.
- **Checking writes.** Inserts, saves, bulk updates and upserts run in a transaction (`atomically!`), and then count that the written rows meet the condition. Counting allowed rows, rather than searching for disallowed ones, means a rule that evaluates to `NULL` refuses instead of admitting.
  - **Upserts** first check that an existing row they'd overwrite is updatable.
  - **Explaining refusals.** A refusal names the first matching `deny`'s message, then the first `allow`'s, then a generic one.
  - **Not found versus forbidden.** A single-row write that matches nothing reports `NotFound` when the request can't see the row, `Stale` for an old version, and `Forbidden` otherwise.
- **Cascades** are integrity rather than access, so they reach linked rows whatever those tables' policies say.
- **Macro.** `#[db::policy]` sits below `#[db::table]`, which consumes it. A misplaced one is a compile error, and an imported `policy` is marked used.
- **Moved out.** RLS mode stays in phase 7, and per-audience projections are in phase 5.

### 4.10 Views (Gel aliases)

```rust
/// Read-only, backed by a SQL view that the migrations manage.
#[db::view(name = "author_stats", public, sql = "
	SELECT author_id, count(*) AS posts, max(created_at) AS last_post_at
	FROM posts WHERE deleted_at IS NULL GROUP BY author_id")]
pub struct AuthorStats {
	pub author_id: Id<User>,
	pub posts: i64,
	pub last_post_at: Timestamp,
}

AuthorStats::query().filter(|s| s.posts.gt(10)).all(&db).await?;
```

- **Named query scopes** (lightweight aliases) are plain Rust: `impl Post { pub fn published() -> db::Query<Post> { … } }`.
- **Materialized views** (Postgres only) use `#[db::view(materialized)]`, refreshed with `AuthorStats::refresh(&db)`.

### 4.11 Annotations, modules, branches, extensions

| Gel                  | leptoskit_db                                                                                                                                                                                                                                     |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Annotations          | Doc comments go to `COMMENT ON` (Postgres), to `schema.json`, to JSON Schema descriptions (and so to the generated client docs), and to the admin studio. `#[db(deprecated = "use display_name")]` warns at compile time on use.                 |
| Modules              | Rust modules organise the code. `#[db::table(schema = "billing")]` maps to a Postgres schema, or to a `billing_` table prefix on SQLite.                                                                                                         |
| Branches             | **A dev database per git branch.** `lk dev` uses `myapp_dev_<branch>` (Postgres, cloned from `main`'s dev database with `CREATE DATABASE … TEMPLATE`) or `.leptoskit/db/dev-<branch>.sqlite`. Switching branches no longer breaks your dev data. |
| Extensions           | **Schema packs** (§7): `leptoskit_auth`, `db::audit`, `db::search`, `db::storage`, each a crate or feature that contributes tables to your migrations.                                                                                           |
| Introspection and UI | `schema.json` is the introspection format. `lk db studio` serves a local data browser, itself built with leptoskit, that respects `private`/`secret` redaction unless you pass `--reveal`.                                                       |
| Future behaviours    | `[db] edition = 1` in `leptoskit.toml`. Behaviour changes ship behind a new edition, and `leptoskit migrate` upgrades it.                                                                                                                        |

### 4.12 Full-text search (Gel's `fts` extension)

```rust
#[db::table(name = "articles", public)]
pub struct Article {
	pub id: Id<Article>,
	#[db(search(weight = A))]
	pub title: String,
	#[db(search(weight = B))]
	pub body: String,
}

Article::query().search("passkeys rust").limit(10).all(&db).await?;   // ranked by relevance
```

Postgres uses a generated `tsvector` column with a GIN index. SQLite uses an FTS5 external-content table kept in sync by triggers.

### 4.13 Geospatial: PostGIS, and a compatible layer for SQLite

**Feature `geo`.** Later phase (8). On Postgres it's backed by PostGIS. On SQLite it's backed by a built-in layer with the same Rust API, explained below.

#### What we keep from gelx, and what we fix

gelx's `Geometry` and `Geography` wrap `geo::Geometry` and encode it as WKB. This plan keeps that foundation (the georust `geo-types`, `geo` and `wkb` crates) and fixes six things:

| gelx today                                                                 | leptoskit_db                                                                                                                         |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| No SRID (a TODO in `geometry.rs`)                                          | The SRID is part of the type: `Geo<Polygon, Srid<27700>>`. It's written as EWKB, and mixing SRIDs fails to compile                   |
| Always the `geo::Geometry` enum, so every read needs a `match`             | Typed columns: `Geography<Point>` is stored as `geography(Point, 4326)`, so the database enforces the shape and Rust gets a `Point`  |
| `Geometry` and `Geography` are near-copies                                 | One codec, generic over the coordinate system                                                                                        |
| `.unwrap()` while encoding; every decode error is `MissingRequiredElement` | Encoding and decoding return `DbError::Decode { column, reason }`; no panics                                                         |
| Values only                                                                | Spatial filters, ordering, KNN and indexes in the typed query builder                                                                |
| serde in geo's internal format                                             | **GeoJSON** on the wire, with a GeoJSON JSON Schema, so generated TypeScript and Dart clients hand it straight to MapLibre or Mapbox |

#### Types (`leptoskit::db::geo`; client-safe, because `geo-types` compiles to wasm)

```rust
pub struct Geography<T>(pub T);                   // lon/lat on the WGS 84 spheroid; distances in metres
pub struct Geo<T, S: CoordinateSystem = Wgs84>(pub T, PhantomData<S>);   // planar, in the SRID's units
pub struct Srid<const CODE: u32>;                 // pub type Wgs84 = Srid<4326>; pub type WebMercator = Srid<3857>;
// T is any geo-types shape: Point, LineString, Polygon, MultiPoint, MultiLineString,
// MultiPolygon, GeometryCollection, Rect, or Geometry for "any shape".
```

**Which to use:**

- **`Geography`** is the right default for "near me", delivery radii and anything on the real globe. Distances are in metres, and nothing breaks at the poles or the antimeridian.
- **`Geo<…, Srid<N>>`** is for projected, planar data, such as national grids, CAD and game maps.

#### Declaring geospatial columns

```rust
#[db::table(name = "places", public)]
pub struct Place {
	pub id: Id<Place>,
	pub name: String,
	#[db(geo(index))]                                     // GiST on Postgres; R*Tree on SQLite
	pub location: Geography<Point>,
	#[db(geo(index, valid))]                              // valid: CHECK (ST_IsValid) plus geo's validation in Rust
	pub delivery_area: Option<Geography<MultiPolygon>>,
	#[db(private)]
	pub survey_boundary: Option<Geo<Polygon, Srid<27700>>>,   // British National Grid
}
```

Constraints work in all four places, as in §4.3:

- `Geography` coordinates are checked for range, longitude −180…180 and latitude −90…90, in Rust, on the client and in a `CHECK`.
- `valid` uses `ST_IsValid` in Postgres and `geo`'s validation in Rust.

#### Querying

```rust
let here = Geography(point!(x: -0.1276, y: 51.5072));

// Within a radius, nearest first, with the distance attached.
let nearby: Vec<(Place, Meters)> = Place::query()
	.filter(|p| p.location.within_distance(&here, Meters(500.0)))
	.nearest(|p| p.location, &here)                       // index-assisted KNN (`<->`) on Postgres
	.with_distance()
	.limit(20)
	.all(&db)
	.await?;

// Map viewport: everything intersecting the visible box.
let visible = Place::query().filter(|p| p.location.intersects(&viewport)).all(&db).await?;

// Which delivery areas cover this address?
let zones = Place::query().filter(|p| p.delivery_area.contains(&here)).all(&db).await?;
```

**Portable operations** behave the same on both databases and are tested against shared fixtures:

- predicates: `intersects`, `contains`, `within`, `disjoint`, `touches`, `within_distance`;
- measurements: `distance_to`, `area`, `length`;
- `bounding_box`;
- `nearest`.

**PostGIS-only operations** exist only when `geo` and `postgres` are both enabled, so a SQLite app can't call them:

- shape operations: `buffer`, `union`, `simplify` (topology-preserving);
- `transform::<Srid<3857>>()`, which uses PostGIS's PROJ;
- `cluster_dbscan`.

Raster, topology and pgRouting stay in raw SQL (`db.sql(...)`), with geo types as parameters and results.

#### On the client

- `PublicPlace.location` serializes as `{"type":"Point","coordinates":[-0.1276,51.5072]}`.
- `geo` algorithms (distance, contains, simplify) are available in wasm behind `geo-algorithms`. A map component can filter or measure locally without a round trip.
- `private` and `secret` apply as usual. Precise locations are often personal data, so `lk db check` warns when a `Geography<Point>` on a public table isn't marked either way.

#### SQLite: the compatible layer

SpatiaLite exists, but it's a native library (`mod_spatialite`) that the deployment has to install. It's awkward on servers, and impractical in the iOS and Android builds where leptoskit's on-device SQLite also runs. Instead, `leptoskit_db` ships its own small layer with the same Rust API:

| Piece            | How it works                                                                                                                                                                                                                                                            |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Storage          | GeoPackage geometry encoding: a `BLOB` holding a small header (SRID and envelope) plus standard WKB. GDAL and QGIS can read the geometries, and full GeoPackage metadata tables are an option (decision G4)                                                             |
| Spatial index    | SQLite's built-in R*Tree module (compiled into the bundled SQLite). An `rtree_<table>_<column>` table holds each envelope, maintained by triggers, so raw SQL writes stay indexed too                                                                                   |
| Exact predicates | SQL functions registered on every connection (`lk_st_intersects`, `lk_st_contains`, `lk_st_dwithin`, `lk_st_distance`, …), implemented in Rust with the `geo` crate. Geodesic distances use Karney's algorithm, matching PostGIS `geography` to well under a millimetre |
| Query plan       | The R*Tree narrows candidates by envelope, and the exact function confirms them. `nearest` grows the search box until it has enough exact candidates, then orders by exact distance                                                                                     |
| Differences      | `geo` and GEOS (which PostGIS uses) can disagree on degenerate boundary cases. The shared fixture suite records every known difference in the docs, and none exist for valid geometries in the portable operations                                                      |

**What the SQLite layer can't do:**

- **Coverage:** only the portable operations, never the PostGIS-only ones.

The same layer can later back geo queries in `leptoskit_plugin_sqlite` on devices, for offline maps.

#### Migrations and development

- **Postgres extension:** the first `geo` column adds `CREATE EXTENSION IF NOT EXISTS postgis` to the migration. Columns become `geometry(<Shape>, <srid>)` or `geography(<Shape>, 4326)`, with GiST indexes for `#[db(geo(index))]`.
- **devenv projects:** the scaffold adds `postgis` to `services.postgres.extensions`.
- **Embedded Postgres** (D6's fallback) usually ships without PostGIS. `lk doctor` detects this and points to devenv or a `postgis/postgis` container.
- **Hosting:** managed Postgres support for PostGIS varies, so `lk db check --live` verifies the extension exists before a deploy applies the migration.

### 4.14 Vector embeddings, on both databases

**Feature `vector`.** Later phase (8). On Postgres it uses `pgvector`; on SQLite it uses `sqlite-vec`, statically linked so nothing needs installing.

#### Types

The dimension is a const generic, so pairing a 768-dimension model with a 1536-dimension column fails to compile:

```rust
pub struct Vector<const N: usize>([f32; N]);         // pgvector `vector(N)`; sqlite-vec float32
pub struct HalfVector<const N: usize>([f16; N]);     // pgvector `halfvec(N)`; sqlite-vec int8 (quantised) or float32
pub struct BitVector<const N: usize>;                // pgvector `bit(N)`; sqlite-vec bit. For binary quantisation
pub struct SparseVector<const N: usize>;             // pgvector `sparsevec(N)` only (Postgres)
```

#### Declaring a vector column

```rust
#[db::table(name = "articles", public)]
pub struct Article {
	pub id: Id<Article>,
	#[db(search(weight = A))]
	pub title: String,
	#[db(search(weight = B))]
	pub body: String,
	#[db(private, vector(distance = cosine, index = hnsw, source(title, body)))]
	pub embedding: Option<Vector<1536>>,
}
```

| Attribute                                                              | Postgres (`pgvector`)                                                                | SQLite (`sqlite-vec`)                                                                                                                       |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Storage                                                                | A `vector(1536)` column                                                              | A `BLOB` column, plus a companion `vec0` virtual table that triggers keep in sync, with the table's key as `rowid` mapping                  |
| `distance = cosine \| l2 \| inner_product \| l1 \| hamming \| jaccard` | Operators `<=>`, `<->`, `<#>`, `<+>`, `<~>`, `<%>` and the matching operator classes | `vec_distance_cosine`, `vec_distance_l2`, `vec_distance_hamming`; the rest computed in Rust where sqlite-vec has none                       |
| `index = hnsw \| ivfflat`                                              | An HNSW or IVFFlat index with the operator class for the distance                    | Exact search, scanning every vector. Fine for small and medium collections; `lk db check` warns when a table grows past a configurable size |
| Filtered search                                                        | Iterative index scans, so filters don't starve the results                           | `vec0` metadata and partition columns for the commonest filters; otherwise an exact scan with `WHERE`                                       |

#### Querying

```rust
let hits: Vec<(Article, f32)> = Article::query()
	.filter(|a| a.published_at.is_not_null())             // policies and filters apply as usual
	.nearest(|a| a.embedding, &query_embedding)
	.with_distance()
	.limit(10)
	.all(&db)
	.await?;

// Hybrid search: full-text ranking fused with vector similarity (reciprocal rank fusion).
let results = Article::query()
	.hybrid(|h| h.text("passkeys in rust").vector(|a| a.embedding, &query_embedding))
	.limit(10)
	.all(&db)
	.await?;
```

#### Keeping embeddings fresh without a built-in AI service

- **`source(title, body)`** stores a hash of the source fields. When a write changes them, the embedding is marked stale. A stale row can still be found, but it's flagged.
- **`Article::stale_embeddings().limit(100)`** gives a background job its work queue.
- **An `Embedder` trait** lets that job call whatever model you choose: local, self-hosted or a provider. No model or service is wired in by default.

**Vectors are private by default.** `lk db check` warns if a vector field is public: embeddings are large, and they can leak the text they were made from.

## 5. Querying

### 5.1 The core API (unchanged from revision 1)

```rust
let posts = Post::query()
	.filter(|p| p.published_at.is_not_null() & p.title.ilike("%rust%"))
	.order_by(|p| p.published_at.desc())
	.limit(20)
	.all(&db)
	.await?;

let user = User::get(&db, id).await?;                 // NotFound error
let maybe = User::find(&db, id).await?;               // Option
NewPost { … }.insert(&db).await?;                     // validated, returns Post
NewTag { name }.upsert(tx).on_conflict(|t| t.name).returning().await?;
user.save(&db).await?;                                // whole row; updated_at bumped; version checked if Versioned
User::update().filter(|u| u.id.eq(id)).set(|u| u.role, Role::Admin).run(&db).await?;
db.transaction(async |tx| { … Ok(value) }).await?;
```

### 5.2 Shapes: Gel's nested selects

EdgeQL's best feature is fetching a nested shape in one query. Here, a shape is an annotated struct:

```text
# EdgeQL
select User { handle, post_count := count(.posts),
              latest := (select .posts { id, title } order by .created_at desc limit 3) }
filter .handle = 'ifiok';
```

```rust
#[derive(db::Shape, Serialize, Deserialize, JsonSchema)]
#[db(from = User, public)]                       // public: may only select public fields; compile error otherwise
pub struct ProfileCard {
	pub handle: Handle,
	pub display_name: String,
	#[db(count(posts))]
	pub post_count: i64,
	#[db(via = posts, order_by = created_at desc, limit = 3)]
	pub latest: Vec<PostTitle>,
	#[db(via = posts, flatten_first, filter = "published_at IS NOT NULL", order_by = published_at desc)]
	pub pinned: Option<PostTitle>,
}

#[derive(db::Shape, Serialize, Deserialize, JsonSchema)]
#[db(from = Post, public)]
pub struct PostTitle {
	pub id: Id<Post>,
	pub title: String,
}

// One SQL statement, policy-filtered at every level:
let card: ProfileCard = User::query()
	.filter(|u| u.handle.eq(handle))
	.shape::<ProfileCard>()
	.one(&db)
	.await?;
```

**How a shape compiles:**

- **Postgres:** `LEFT JOIN LATERAL` subqueries with `json_agg`.
- **SQLite:** correlated subqueries with `json_group_array`.
- **Both:** a single round trip with no N+1. The decoder is generated, so no `row.get("…")` appears in your code, unlike openbudget's `TokenPopulated`.

**Shapes are also the safe way to return joined data to clients.** A `public` shape that names a `private` field fails to compile, and policies apply to every nested table.

### 5.3 Escape hatch

```rust
#[derive(db::Row)]
struct Revenue { day: NaiveDate, total: Money }
let rows: Vec<Revenue> = db.sql(include_str!("sql/revenue.sql")).bind(org_id).fetch_all().await?;
```

Raw SQL on a request-scoped `Db` still runs under RLS when it's enabled. Without RLS, raw SQL is not policy-filtered, and the docs say so.

## 6. Migrations

Revision 1's model is unchanged:

- the exporter compiles the structs to a target schema;
- the diff runs against the last committed `schema.json`, offline and deterministically;
- the result is `migration.sql` plus a snapshot plus `registry.txt`, applied forward-only with checksums;
- renames prompt, or need `renamed_from`, and destructive changes need a flag;
- `lk db check` guards CI.

This revision adds:

- **Schema packs in your history.** Pack tables, such as `auth.*`, are part of your exported schema. When `leptoskit_auth` changes its tables, your next `lk db migration create` includes those changes, and you review them like your own. The history stays linear, with no hidden migrations.
- **New objects the diff understands:** policies (RLS mode), views, generated columns, partial indexes, `CHECK` constraints, search and vector indexes, typed PostGIS columns with GiST indexes, SQLite R*Tree and `vec0` companion tables with their triggers, polymorphic supertype tables, comments, and Postgres extensions (`CREATE EXTENSION IF NOT EXISTS vector`).
- **Portable mode** writes one directory per dialect from the same plan. CI applies both to real databases.
- **Postgres safety:**
  - Indexes on existing large tables use `CREATE INDEX CONCURRENTLY`. The migration is split into a non-transactional step, and the file says so.
  - Adding `NOT NULL` uses `ADD COLUMN … DEFAULT` followed by a validated constraint.
  - `lk db migration create` prints lock warnings for operations that rewrite a table.
- **Data migrations in Rust** (decision D7), alongside SQL ones: `lk db migration create backfill-handles --rust` creates `migrations/<id>/migrate.rs`. It's compiled into the server through `migrations!()` and runs in the migration's transaction with a `system()` `Db`.
- **Branch databases** (§4.11). When git switches branch, `lk dev` switches dev database; `lk db branch prune` deletes merged ones.

## 7. Schema packs (Gel's extensions, Serverpod's modules)

A pack is a crate that ships tables, policies, server functions and headless client hooks:

```rust
// in the app's db crate
leptoskit::db::packs! {
	leptoskit_auth::pack(),      // accounts, identities, sessions, passkeys, … (auth-plan.md)
	db::audit::pack(),           // _leptoskit_audit
	db::storage::pack(),         // file metadata; blobs in S3/R2/disk
}
```

**Pack rules:**

- **Namespacing:** pack tables live in a schema or prefix (`auth.sessions`, or `auth_sessions` on SQLite).
- **Read-only to the app:** apps can link to pack tables (for example `Id<auth::Account>`) and read them under the pack's policies. Only the pack's own API writes to them.
- **Portable:** packs must use the portable subset, and every pack is tested on Postgres and SQLite in this repo's CI.

**As built (phase 4, packs):**

- **Declaring a pack table.** `#[db::table(…, pack = "auth")]` marks a table as a pack's:
  - its namespace defaults to the pack's name;
  - `Table::PACK` names the pack;
  - the generated writes (`insert`, `upsert`, `save`, `delete`, `destroy`, `update()`) are `pub(crate)`, so only the pack's crate writes, while reads stay `pub`. A trybuild test pins the error an app gets for writing.
- **Linking packs.** `packs!(crate, …)` expands to `extern crate … as _;`. A dependency nothing references isn't linked, so without it a pack's tables would vanish from the schema, and the next migration would drop them.
- **Portability.** `schema()` runs `Schema::validate_portable` on each pack's tables as a group, so every pack table must:
  - validate on both dialects;
  - link only within the pack;
  - avoid raw SQL defaults and checks.
- **Moved out.**
  - Bulk `Query::delete`/`destroy` on a pack table aren't restricted yet, since they hang off the shared `Query` type. Packs should guard them with policies.
  - Packs-as-values (`leptoskit_auth::pack()` carrying server functions and hooks) waits for `leptoskit_auth`, which will show what a pack needs beyond tables.

## 8. Live data

This is unchanged from revision 1: invalidate, don't replicate. Policies make it safer still, because a refetch runs the same policy-filtered query. Revision 2 adds:

- **Policy-aware event filtering:** an event only goes to subscribers whose policies could see the old _or_ the new row, so the existence of hidden rows doesn't leak.
- **`LISTEN/NOTIFY` fan-out on Postgres**, for running more than one server.

## 9. Layout

### In this repository

```text
crates/
  leptoskit_db/            # runtime: Id, Column, scalars, query + shapes, policies, migrator, live feed
  leptoskit_db_macros/     # table, link, mixin, polymorphic, view, policy, Shape, Scalar, Enum, Row, packs!, migrations!
  leptoskit_db_schema/     # pure: schema model, diff, renderers (postgres, sqlite), snapshot format
  leptoskit_plugin_db/     # `lk db …`, exporter, studio, branch databases, dev-watch
  leptoskit_auth/          # schema pack + server + headless client (auth-plan.md)
  leptoskit_idp/           # OAuth 2.1 / OpenID Connect provider (auth-plan.md)
```

### A complete app

```text
my_app/
├── leptoskit.toml                 # [db], [auth]
├── devenv.nix                     # services.postgres for dev (scaffolded; see D6)
├── src/main.rs
├── migrations/
│   ├── registry.txt
│   ├── 20261028T0915_initial/{migration.sql, schema.json}
│   └── 20261102T1200_add_posts/{migration.sql, schema.json, migrate.rs}
├── my_app_db/
│   └── src/
│       ├── lib.rs                 # packs!, migrations!, globals
│       ├── scalars.rs             # Handle, …
│       ├── mixins.rs              # Audited, …
│       ├── models/{user.rs, post.rs, tag.rs, actor.rs}
│       ├── shapes/{profile.rs, feed.rs}
│       ├── views.rs
│       └── seed.rs
├── my_app_backend/src/{posts.rs, profile.rs}         # server functions; mostly shapes in, Public* out
├── my_app_app/src/
│   ├── routes/auth/{sign_in.rs, verify.rs, mfa.rs}    # generated by `lk generate auth`, yours to restyle
│   ├── routes/settings/security.rs                    # passkeys, TOTP, sessions, connected accounts
│   └── …
└── .leptoskit/db/                                     # exporter, sqlite dev files, push-mode drafts
```

```toml
# leptoskit.toml
[db]
dialect = "postgres"
crate = "my_app_db"
migrations = "migrations"
url-env = "DATABASE_URL"
policies = "orm" # or "rls" (Postgres)
branch-databases = true
edition = 1
```

## Feature map

Tier 1 is the first usable release, tier 2 makes it Gel-class, tier 3 adds power features, and tier 4 adds geospatial and vector support.

| Feature                                    | Gel equivalent                       | Tier | Notes                                  |
| ------------------------------------------ | ------------------------------------ | ---- | -------------------------------------- |
| Tables, `Id<T>`, scalars, enums, `Public*` | object types, properties             | 1    |                                        |
| Single links, backlinks, `on_delete`       | links, backlinks, deletion policies  | 1    |                                        |
| Constraints in four places                 | constraints                          | 1    | `regex` is app-side on SQLite          |
| Custom scalars                             | `scalar type … extending`            | 1    |                                        |
| Mixins via `flatten`                       | abstract types, multiple inheritance | 1    |                                        |
| Built-in rewrites (`updated_at`, …)        | mutation rewrites                    | 1    |                                        |
| Migrations, check, status, packs           | migrations, extensions               | 1    |                                        |
| Access policies (ORM-enforced), globals    | access policies, globals             | 1    | Needed by auth; that's why it's tier 1 |
| Multi links and link properties            | multi links, link properties         | 2    |                                        |
| Shapes                                     | EdgeQL shapes                        | 2    |                                        |
| Hooks and triggers, audit                  | triggers, rewrites                   | 2    |                                        |
| Views, generated columns                   | aliases, computeds                   | 2    |                                        |
| Projections per audience                   | access policies + computeds          | 2    |                                        |
| Branch databases                           | branches                             | 2    |                                        |
| Polymorphic enum tables                    | abstract link targets                | 3    |                                        |
| Postgres RLS mode                          | (database-enforced policies)         | 3    |                                        |
| Full-text search                           | `fts` extension                      | 3    |                                        |
| Geospatial (`geo`)                         | `postgis` extension                  | 4    | PostGIS; compatible layer on SQLite    |
| Vector embeddings (`vector`)               | `ai` extension (storage and search)  | 4    | pgvector; sqlite-vec                   |
| Studio                                     | Gel UI                               | 3    |                                        |

## Delivery

Each phase is one PR, or a `gh stack` of them. Auth phases interleave from phase 4 (see [auth-plan.md](auth-plan.md#delivery)).

0. **Spike:** the macro produces the row, `Public*`, the client marker and a descriptor; a wasm build pulls in no driver; `inventory` export works.
1. **`leptoskit_db_schema`:** the model, diff and renderers for Postgres and SQLite. The snapshot tests include constraints, partial indexes and comments.
2. **Runtime (Postgres and SQLite):** CRUD, upsert, transactions, scalars, enums, mixins, rewrites, constraints with validation, errors, migrator, and `db::testing` (Postgres template databases, SQLite in memory).
3. **Tooling:** `lk db …`, `[db]` config, the scaffold (Postgres by default, dev Postgres per D6), dev-watch, docs, changesets. **This is the first release.**
4. **Policies and globals, plus the pack mechanism.** This unblocks `leptoskit_auth`.
5. **Shapes, multi links, hooks and audit, views, generated columns, projections.**
6. **Branch databases, push-mode dev loop, live invalidation.**
7. **Polymorphic tables, RLS mode, full-text search, studio.**
8. **Geospatial and vectors** (tier 4), in two stacked PRs:
   - `geo`: the types and EWKB/GeoJSON codecs; PostGIS columns, operations and GiST; the SQLite layer (GeoPackage encoding, R*Tree triggers, registered `geo` functions); a shared fixture suite run on both databases;
   - `vector`: pgvector types and indexes; sqlite-vec static linking and `vec0` companion tables; `nearest`, `hybrid`, stale tracking and the `Embedder` trait.

## Decisions

Revision 1's decisions stand, except that "one dialect per project" now has a portable-mode exception (D2).

| #  | Decision                                                  | Recommendation                                                                                                                                                                         | Trade-off                                                                                                                                                                                                               |
| -- | --------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1 | MySQL                                                     | **Decided: not supported.** Postgres and SQLite only                                                                                                                                   | Postgres and SQLite cover single-box through scale, and dropping MySQL removes a fallback path from every feature                                                                                                       |
| D2 | Portable mode for apps                                    | Available, off by default; always on for packs                                                                                                                                         | Lets an app run on SQLite at the edge or locally and on Postgres at scale, at the cost of using only the portable subset                                                                                                |
| D3 | Policy enforcement                                        | ORM-level everywhere; Postgres RLS as an opt-in second layer                                                                                                                           | ORM-level is portable and gives better errors. RLS also covers raw SQL but needs `SET LOCAL` per transaction and is harder to debug.                                                                                    |
| D4 | Projections per audience                                  | Yes, tier 2                                                                                                                                                                            | One more generated type per projection, but email-visible-to-self is unavoidable in real apps                                                                                                                           |
| D5 | Polymorphism as Rust enums                                | Yes, using class-table inheritance                                                                                                                                                     | Exhaustive matches are better than Gel's `[is Type]`. It costs one join, or one query per variant, when loading through the supertype.                                                                                  |
| D6 | Dev Postgres                                              | **Decided:** devenv `services.postgres` when the project uses devenv; otherwise `lk dev` runs an embedded Postgres (binaries cached under `~/.cache`)                                  | Embedded Postgres means a one-time binary download. Docker is the other option, but it's heavier and not always present.                                                                                                |
| D7 | Rust data migrations                                      | Yes, tier 2                                                                                                                                                                            | Backfills often need app logic (hashing, parsing), but compiled migrations make old migrations depend on today's code, so they're limited to `system()` queries and plain types                                         |
| G1 | SQLite geo backend                                        | Our own compatible layer, with SpatiaLite not supported                                                                                                                                | SpatiaLite covers far more functions, but every deployment needs a native library, and it doesn't fit iOS, Android or on-device use. The portable operations cover typical app needs; heavy GIS work belongs on PostGIS |
| G2 | Registering SQLite functions (geo predicates, sqlite-vec) | **Decided during phase 2:** the server SQLite driver is `rusqlite`, whose safe API registers functions and extensions, so no `unsafe` module is needed. Postgres uses `tokio-postgres` | sqlx's SQLite driver needs an older `libsqlite3-sys` than the on-device plugin's `rusqlite`, and Cargo allows one SQLite per build, so sqlx was dropped                                                                 |
| G3 | SRIDs in the type system                                  | Yes: `Geo<T, Srid<N>>` and `Geography<T>`                                                                                                                                              | Mixing coordinate systems is the classic silent GIS bug. It costs one type parameter, which defaults to WGS 84                                                                                                          |
| G4 | Full GeoPackage metadata on SQLite                        | No by default; `[db.geo] geopackage = true` writes the `gpkg_*` tables                                                                                                                 | The default keeps the schema lean. Opt in when the file must open in QGIS as a layer                                                                                                                                    |
| G5 | Wire format for geometry                                  | GeoJSON (RFC 7946)                                                                                                                                                                     | Map libraries consume it directly. It's verbose for big polygons, so a `Wkb` wrapper type opts into base64 WKB                                                                                                          |
| D8 | Policies in attributes vs. `impl Policy`                  | Attributes for short rules; `impl db::Policies for Post` for long ones; both compile to the same expressions                                                                           | Attributes keep rules next to the data; long closures in attributes hurt rustfmt and readability                                                                                                                        |

## Risks

- **Scope.** Revision 2 is several times larger than revision 1. The tiers are the guardrail. Tier 1 alone has to feel finished: tables, constraints, mixins, migrations and policies.
- **Policy correctness is security-critical.** We need property tests that compare policy-filtered results with a reference implementation of each policy, plus tests for the RLS mode.
- **Shapes compile to complex SQL.** Snapshot-test the generated SQL and run it against both dialects; budget for tuning query plans.
- **Proc-macro error quality.** Every new attribute needs `syn::Error` spans and trybuild tests.
- **Welds 0.5.x is still unread.** Check it before vendoring its query-builder pieces.
- **Geo results that differ between databases.** The SQLite layer and PostGIS run different geometry engines (`geo` and GEOS). One fixture suite runs on both, and any divergence is either fixed or documented before release.
- **Extension availability.** PostGIS and pgvector have to exist on the target Postgres. `lk db check --live` verifies them before a deploy, and `lk doctor` catches a dev setup without them.
- **sqlite-vec scale.** Exact search slows as tables grow. The size warning makes that visible, and the docs say when to move to Postgres.
