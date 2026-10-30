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

**As built (phase 5, multi links):**

- **A link is a table.** `#[db::link(…)]` is `#[db::table(…)]` with a hidden first argument (`__link`) that survives mixin callbacks. The table's expansion inserts the two ends first, as `#[db(key, on_delete = cascade)] pub <end>_id: Id<End>`, so the key is `(from, to)`; it indexes the `to` column, or makes it unique with `exclusive`. Everything else is a table's: policies (`#[db::policy]` below `#[db::link]`), mixins, migrations, `by_post`/`post()` on the join row, packs. A link may not declare its own key, a field named like an end, or `unique` without columns.
- **Columns.** An end's column is its table's last path segment in snake case plus `_id`; `from_column`/`to_column` rename them, and a link from a table to itself must, since both would be the same.
- **Writing it as `db::link`.** A bare `#[link]` is Rust's built-in FFI attribute and can't be shadowed by an import, so the docs always write `db::link`. The runtime's `link` module became `links` so the macro's re-export isn't shadowed.
- **No `unique` flag.** Gel's multi links are sets, so a pair is linked at most once by the key; `unique` from the sketch is rejected with an explanation, and `add` on an existing pair upserts, replacing the link's properties and keeping `created_at`.
- **Runtime.** `Link` (`From`, `To`, `Properties`, `FROM`, `TO`, `link`, `source_id`, `target_id`), and `Links<J, Near, Far>` built by `Links::forward(id)`/`Links::backward(id)` and by the generated `post.tags()`/`tag.posts()`, which, like backlinks, need both tables in the link's crate. Reads: `all` (each far row with its link, two statements), `link_rows`, `ids`, `count`, `contains`; narrowing with `filter` (far rows), `filter_link`, `order_by` (link columns, default the far key), `limit`, `offset`. Writes: `add(conn, far, properties)` and `remove(conn, far)`, only for `WritableLink`, which every link but a pack's implements; `{Link}Properties` holds the insert's columns minus the ends, or the link takes `()`.
- **Both ends visible (decision).** Reads and `remove` keep only links whose near and far rows the request may see and that aren't soft-deleted, through `Expr::Within` (`column IN (SELECT key FROM end AS lk_in_<n> WHERE policy AND deleted_at IS NULL)`), skipped when an end has neither. `add` first reads both ends through policies in the same transaction and fails with `NotFound`. This is Gel's behaviour (a link to an invisible object isn't there) and fails closed; the link table's own policies apply on top. `Count` and `Within` share `policy::Subquery`.
- **Moved out.** The `links(tags: Tag)` shorthand on `#[db::table]` and `on_source_delete = delete_orphan_targets` (see "As built (phase 6)" below). Ordering `all` by the far table's columns: sort in Rust, or use a shape.

**As built (phase 6, replacing a link set):** `Links::set(conn, [(far, properties), …])` runs in `atomically!`: the near row must be visible, each far row must be visible (as `add`), duplicates and a `limit`/`offset` on the `Links` fail with `DbError::Encode`. It deletes the visible links (both ends visible, the `Links`' `filter`/`filter_link` narrowing them, `select` policy) to rows not in the set, through `Query::delete`, or one by one through `write::delete` when the link has `after_delete`; when fewer were deleted than were visible, the delete policy kept one, and the whole set fails with `Forbidden` through `policy::refuse`. Then it upserts each pair in order and returns the stored links. Links to rows the request can't see are never touched (decision: fail closed rather than delete what it can't see). `tests/multi_links.rs` covers it on both databases.

**Not built (phase 6), and why:**

- **`links(tags: Tag)`.** It saves one line over `#[db::link(name = "post_tags", from = Post as tags, to = Tag as posts)] pub struct PostTag {}`, but has to invent a public struct name (`PostTags`?), a table name that migrations, policies and `schema.json` then carry, and a backward method name, none of which the user wrote. Renaming any of them later is a migration. The explicit form is short and says all three; the shorthand is a poor fit.
- **`on_source_delete = delete_orphan_targets`.** Now possible on top of `fallout` (below), but "orphan" needs a decision on soft deletes (a target linked only from soft-deleted sources: orphaned or not?), on targets linked through _other_ link tables, and on policies (may a request delete a target it can't see because its last link went?). It needs a registry of links by source, like `SoftCascade`, run on hard and soft deletes. Left for a later phase with those answers.
- **Tests.** `tests/multi_links.rs` (both databases): adding, reading, and removing both ways, re-adding, paging, self-links, `exclusive`, soft deletes, cascades, and link policies, plus a proptest of what each request sees through links against a plain-Rust model (96 cases on SQLite, 48 on Postgres). Dropping either end's visibility filter fails it on both. The pack fixture checks a pack's link is read-only to apps (trybuild).

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

**As built:** `Email`, `Url` and `Slug` shipped with the derive. A scalar wraps one `String` or number, so the rest moved: `NonEmpty<String>` is `min_len = 1` on a text scalar; `Money` has two columns, so it is a mixin (§4.5); and `Pubkey` and `Signature` landed with Solana wallets in phase Auth 4, behind `leptoskit_db`'s `solana` feature, using a new `validate_with` derive option for their decoded-length check. `parse` takes `impl Into<String>` for text and the exact type for numbers, so integer literals infer.

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
- **Moved out.** `HasMixin` gives generic code the mixin's values. Typed column handles per mixin (`t.mixin::<Timestamps>().updated_at`) are left for later, since `T::UPDATED_AT` covers the common case. `#[db(created_by)]` and `#[db(updated_by)]` landed in phase 5, with hooks (§4.8).

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

**As built (phase 7):**

- **Declared on both sides.** The enum lists its variants and each variant table says `#[db::table(extends = Actor)]`, as Gel's `type User extending Actor` does. The compiler checks both directions: the table's generated `write_supertype` needs `User: Variant<Of = Actor>`, which only the enum implements, and the enum asserts in a `const` that each listed table has a `SUPERTYPE`. Both sides are in one crate, since the enum implements `Variant` for each table; a pack's table can't be an app's variant.
- **The supertype table** is `{Enum}Row`, generated through `#[db::table]`, keyed by `Id<Actor>` (`generate = false, no_foreign_key`) so that supertype rules compare its key with other `Id<Actor>` columns and settings. `{Enum}Kind` is a `db::Enum` of the variants in `snake_case`, or `#[db(rename)]`.
- **`Id<T>` columns** now need `T: Referenced` rather than `T: Table`: every table implements it, and so does the enum, naming the supertype table. Link accessors and forward relations need `Linkable` (every `HasId` table, and the enum), whose `load` reads the supertype row for the kind and then that variant's row. Backlinks to the enum work; shapes can't select it, because its rows span tables (a trybuild test pins the error).
- **Reads through the enum** are one query per variant (`Variants<P>`), not a join: each variant's query is the table's own `Query`, so its policies, soft deletes and row-level security apply unchanged, with no second code path to keep in step. Paging merges in key order: each table returns its first `offset + limit` rows, and the merge keeps the page. Ordering is by key only, since the supertype has no other columns.
- **Policies.** `policy::condition::<T>` ANDs the table's own rules with `id IN (SELECT id FROM actors WHERE <supertype rule>)`. For writes, the subquery also requires the supertype's `select` rule, matching what Postgres row-level security does to a subquery inside a policy, so ORM and RLS mode agree; the property test in `tests/polymorphic.rs` checks reads against a model, and a model without the supertype's rule is shown to fail it. Refusals explain themselves with the supertype's messages too.
- **Writes.** Inserts and upserts that insert write the supertype row first, in the same transaction; under row-level security it is written without `RETURNING`, since Postgres refuses to return a row the request can't read before the variant's own check can explain the refusal. Single and bulk removals delete the supertype rows after the variant rows and fail, rolling back, if a policy keeps one. The variant's key is `ON DELETE CASCADE` to the supertype, so removing a supertype row directly removes the variant row (and the fallout audit records it).
- **Soft deletes** leave the supertype row, so links stay valid; `on_soft_delete = cascade` links to the enum follow a soft delete of any variant, single or bulk.
- **Live.** `#[db::polymorphic(live)]` makes the supertype table live: adding or removing a row of any kind publishes `actors` with its key, to subscribers whose `select` rules allow the supertype row. Edits to a variant row publish only that variant's table.
- **Moved out.** `Id<User>` → `Id<Actor>` is `upcast()`, not `.into()`: a blanket `From<Id<V>> for Id<V::Of>` would overlap core's reflexive `From`. Shapes over the enum, and ordering by anything but the key, are left for later.

### 4.7 Computed values

Gel computeds are evaluated at query time. There are three Rust equivalents:

| Need                                             | Use                                                                                                                                 |
| ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| Derived in memory only                           | A plain Rust method on the row or on `PublicX`                                                                                      |
| Filterable or indexable, derived from columns    | `#[db(generated = "lower(handle)")] pub handle_key: String`, a stored generated column (read-only; excluded from `NewX` and `save`) |
| Derived from other tables (counts, latest child) | A field on a **shape** (§5.2): `#[db(count(posts))] post_count: i64`                                                                |

`generated` takes dialect SQL. In portable mode, write `generated(postgres = "…", sqlite = "…")`.

**As built (phase 5, generated columns):**

- **Model.** `Column::generated: Option<Generated { postgres, sqlite }>`, absent from snapshots when `None`, so existing `schema.json` files read unchanged. `#[db(generated = "…")]` fills both; `generated(postgres = "…", sqlite = "…")` sets each. Both dialects render `GENERATED ALWAYS AS (…) STORED`. Validation refuses a default, a generated key, a foreign key from a generated column, and an empty expression; packs may not have one (raw SQL).
- **Diff.** Adding one needs no default (the database fills existing rows). Changing a generated column's expression on the project's dialect, or whether a column is generated, is refused with `DiffError::GeneratedChanged`: Postgres can't change it in place everywhere (`SET EXPRESSION` is 17+), and dropping it would drop its indexes, which the index diff wouldn't see. Dropping one is `Risk::Safe` (derived data, so no `--allow-destructive`), and it sorts with `AddColumn`, before `AlterColumn` and other drops, because Postgres refuses to retype or drop a column a generated column reads. SQLite can't `ADD COLUMN … STORED`, so adding one rebuilds the table, and a rebuild never copies generated columns; any `DROP COLUMN` on a table with one rebuilds too, since SQLite refuses to drop a column a generated one reads and the expression isn't parsed. Scenarios for adding, dropping beside, dropping with its input, and retyping an input apply against both databases in `leptoskit_db_schema`'s apply tests.
- **Runtime.** `Table::GENERATED` lists them. `INSERT` and upserts' `DO UPDATE SET` leave them out, `save` leaves them out and, on a table with any, uses `RETURNING` to replace the row with what was stored (version included, so it isn't bumped twice), and `Update::set` on one fails with `DbError::Encode`. `Table::validate` skips them, the insert type omits them, and `into_row` fills a `Default` placeholder, so their types need `Default`.
- **Macro.** A generated column may not be the key, a link, a timestamp or version, a flattened mixin, or have a `default` or rules (`min_len`, …), each with its own message. Being in a mixin is allowed.
- **Moved out.** `VIRTUAL` columns on SQLite, and computed fields on a shape from SQL expressions (counts are there; arbitrary aggregates aren't).

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

**As built (phase 5, hooks, rewrites, and the audit log):**

- **Declaring hooks.** `#[db::table(…, hooks(before_write, after_insert, after_update, after_delete))]` names them, and the table implements `BeforeWrite`, `AfterInsert`, `AfterUpdate`, `AfterDelete` (each with `#[diagnostic::on_unimplemented]`). Rust can't detect an impl without specialization, so the list is what tells the macro to generate `Table::hook_*` methods that delegate to the traits (named `hook_*` so they never clash with the traits' own methods). `Table::HOOKS: Hooks` records which exist.
- **Signatures (decision).** `before_write(&mut self, cx: &WriteContext<'_, Self>) -> Result<()>`; `after_insert(&self, tx)`, `after_update(old, new, tx)`, `after_delete(&self, tx)`, each `tx: &impl Connection` returning `impl Future + Send`, so apps write `async fn`. The sketch's `&db::Tx` would need `Tx` to exist without a driver and a way to recover a `Tx` from a generic connection; `&impl Connection` is the transaction at run time either way. `WriteContext` has `is_insert`, `is_update`, `old`, `context`, `actor`, `reason`.
- **Where they run.** Every single-row write with an effect (a hook, an audited table, or a slug) runs in `atomically!`, so hooks join the caller's transaction or open one. `insert`: stamp, `before_write`, slugs, stamp again, validate, write, verify policy, audit, `after_insert`. `save`: reads the stored row within the update policy (`SELECT … FOR UPDATE` on Postgres; SQLite's `BEGIN IMMEDIATE` already serializes), `NotFound`/`Stale`/`Forbidden` through `policy::missing` when it's not there, then `before_write` with the old row, write with `RETURNING`, verify, audit, `after_update(old, stored)`. `upsert` reads and locks the conflicting row first: with one it's an update (policy checked before the hook), without one it's a plain `INSERT`, so a concurrent insert fails as a conflict rather than running insert hooks for an update. A hook that changes the key (`save`) or the conflict target (`upsert`) fails with `DbError::ReadOnly`. Soft deletes return the row with `RETURNING`; destroys use `DELETE … RETURNING`, so `after_delete` gets the row as the delete left it. `before_write` doesn't run on deletes.
- **Bulk writes.** `Update::run` fails with `DbError::HooksSkipped` when the table has `before_write` or `after_update`; `Query::delete`/`destroy` when it has `after_delete`; `.allow_skipping_hooks()` on either lets them run. `Links::remove` on a link table with `after_delete` deletes each matching link through `write::delete` (same filter: visible ends and the delete policy), so its hook runs; `add` goes through `upsert` already.
- **Cascades never run hooks.** A table with `after_delete` may not have `on_soft_delete = cascade` (compile error); rows the database removes through `ON DELETE CASCADE` can't run hooks either, and the docs say so.
- **`created_by`/`updated_by`.** `Actor(Uuid)` is a global (`Context::actor()`), with `Actor::of(id)`, `id::<T>()`, `uuid()`. `leptoskit_auth::context` adds `Actor::of(signed_in.account)` beside `SignedIn`, so `leptoskit_db` stays independent of auth. `Access::System` now carries the actor of the handle it was made from, so `request_db.system("…")` still records who asked (`Db::actor()`). Columns implement `ActorColumn` (`Id<T>`, `Uuid`, and both in `Option`); the macro generates `Table::stamp`, which fails with `DbError::NoActor` for a required column with nobody acting. They're left out of `NewX` (`ActorColumn::unset()` until written), re-stamped after `before_write`, excluded from `save`'s and upserts' `SET` for `created_by`, set in bulk updates and soft deletes for `updated_by` (`NULL` or `NoActor` by the column's nullability), and refused in `Update::set` (`ReadOnly`). Soft-delete cascades set the linked table's `updated_by` (or `NULL`, which a `NOT NULL` column rejects in the database).
- **Slugs.** `#[db(slug(from = title), unique)]` (unique, or `unique(except = …)`, is required; the source must be a field the insert provides). `Slug::from_text` (ASCII letters and digits, hyphens between, ≤ 80 characters, `untitled` when empty) fills a placeholder in `into_row`; inserts recompute it from the source after `before_write`, then pick the first free `-N` suffix by reading every row whose slug is `base` or `LIKE base-%` (no policy: uniqueness is over all rows; a partial index's predicate is applied). Saves and upserts keep it (upserts leave slug columns out of `DO UPDATE SET`). Concurrent inserts can collide and fail with `Conflict`.
- **The audit log.** `#[db::table(audit)]` sets `Table::AUDIT`; `schema()` adds `_leptoskit_audit` (`AuditEntry`: `id` (UUIDv7), `at`, `table_name`, `row_key` (the key as a JSON array), `op` (`AuditOp`: insert, update, delete), `actor`, `reason`, `before`, `after` (`Json`), indexed on `(table_name, row_key)` and `at`) only when some table is audited. Column names differ from the sketch (`table`, `key`) to avoid SQL keywords. It's declared with `#[table]` plus a hidden `__internal` flag: not registered in `inventory`, writes `pub(crate)` with `allow(dead_code)`.
- **What's recorded.** Single-row writes record the stored rows; bulk updates and soft deletes read and lock the matching rows, update exactly those keys in batches of 500 with `RETURNING`, and record each pair; bulk destroys record `DELETE … RETURNING`; soft-delete cascades record each linked row of an audited table (the `SoftCascade` entry carries an `audit::Subject`). Entries are inserted 200 per statement. Rows are JSON with `secret` columns left out entirely; `private` ones are kept (decision: the task asked to drop only `secret`, and the log has its own read policy). Every raw row is decoded through the table's type first (`Subject::normalize`): SQLite returns UUIDs from `RETURNING` as bytes, which the property test caught.
- **Reading it fails closed.** `AuditEntry`'s only policy is `select` through an `AuditReader` global (`everything()` or `matching(fn(&AuditEntryColumns, &Context) -> Expr)`), so a request without one reads nothing and no request may insert, change, or delete entries; a `system` handle can purge (retention). `AuditEntry::for_row(&row)` and `for_table::<T>()` query one history.
- **Not recorded.** Raw SQL. Rows the database removes or nulls through `ON DELETE CASCADE`/`SET NULL` are recorded since phase 6 (below).

**As built (phase 6, the database's own cascades):** `fallout.rs` builds, once, a graph of every registered table's cascading (`Cascade`, `SetNull`) foreign keys from `TableEntry` definitions, and whether a hard delete of a table reaches, through cascades, a table that records changes (`audit` or `live`; `SetNull` ends a path). When it does, `write::destroy` and `Query::destroy` (so also `Links::remove` and `Query::delete` on tables without soft deletes) run in `atomically!`: they read and lock the rows to delete (`SELECT … FOR UPDATE` within the delete policy; bulk destroys then delete exactly those keys in batches of 500), walk the graph breadth-first reading and locking each reached row (normalized through its table's type, so SQLite bytes and UUIDs compare), with already-deleted rows skipping cycles, then run the delete and record each reached row of a recording table: a cascade as `Delete` with no `after`, a `SET NULL` as `Update` with the key columns emptied (a row both emptied and deleted is deleted). Locking the parents first keeps a concurrent insert of a child from slipping past the walk. Intermediate tables that record nothing are walked but not recorded. Tested in `tests/live.rs` (two levels of cascade, `SET NULL`, single and bulk destroys, audited on both databases).

- **Tests.** `tests/hooks.rs` and `tests/audit.rs` (both databases): every hook on every write path, rollbacks from failing hooks and inside callers' transactions, upsert branches, refusing key and target changes, the bulk guard, link removal, hooks under policies, actor columns (required and optional, through `system`), slugs; and every write path recorded, refusals and rollbacks recording nothing, cascades, reading policy. A proptest compares the log with a plain-Rust model over random histories of inserts, saves, soft deletes, bulk updates, purges, and system saves by three actors, plus each actor's `AuditReader::matching` view (96 cases on SQLite, 48 on Postgres). Recording secret columns, letting every request read the log, or skipping bulk updates each fails it on both databases.

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

**As built (phase 7, RLS mode):**

- **Settings (decision).** A rule written with `cx.global(|viewer: &Viewer| …)` runs Rust with the request's value, so no database can repeat it, and evaluating closures with placeholder values would silently mis-render any rule that branches. Rules that should run as RLS read the request through `Setting` (`NAME`, `type Value: Column`, `value()`): `cx.setting::<G>()` returns a `SettingValue<V>` operand, and comparisons with it build `Expr::CompareSetting`, written as a bound value (or `NULL` without one, so it and its negation match nothing, like Postgres's `NULL`) and, in a policy, as `NULLIF(current_setting('leptoskit.<name>', true), '')::<type>`. `register_setting!(G)` submits a `SettingEntry` through `inventory`, which is how a transaction knows what to set. `leptoskit_auth`'s `SignedIn` is the setting `account` (`Id<Account>`), and the auth tables' rules use it.
- **Rendering.** `policy::row_security::<T>()` evaluates `T`'s rules for each action with a symbolic `Context` that records what they read: `get`/`global` record the type and return `None`, `setting` records the setting. It refuses rules that read globals, read unregistered settings, or count their own table (Postgres recurses where the ORM's `Reading` skips), and writes each action's condition with `Sql::policy` (Postgres, values inline as escape-string literals, the table's own name as the root scope). `schema_with_row_security(dialect)` attaches them as `Table::row_security` (also checking setting names and duplicates); `export_schema!` uses it when `LEPTOSKIT_DB_POLICIES=rls`, which the CLI sets from `[db] policies = "rls"` (`DbPolicies`, default `orm`, left out of `leptoskit.toml`). Views and the audit log (whose `AuditReader` is a function) aren't covered.
- **Model and migrations.** `RowSecurity { policies: [RowPolicy { command, using, check }] }`, absent from snapshots when `None`; validation refuses it on SQLite and checks each command has the clauses it needs, once. `Step::DropRowSecurity` (first phase) and `Step::CreateRowSecurity` (with `CreateSearch`), recreated when the policies change and whenever any table changes, since a policy's SQL isn't parsed and Postgres won't retype or drop a column one reads. Postgres renders `ENABLE` and `FORCE ROW LEVEL SECURITY` (apps usually connect as the owner) and `CREATE POLICY <table>_<command> … USING (current_setting('leptoskit.bypass', true) = 'on' OR (…))`, `WITH CHECK` for insert and update; dropping the last policies disables it. Scenarios `add_row_security`, `row_security_follows_table_changes` (retyping a column a policy reads), and `drop_row_security` apply on Postgres; the apply test compares `pg_policies` and the RLS flags.
- **Soft deletes (decision).** The ORM soft-deletes with an `UPDATE` checked against the delete rules; Postgres checks every `UPDATE` against the update policy, so on a table with `DELETED_AT` that policy is `USING (update OR delete)` and `WITH CHECK (update OR (delete AND deleted_at IS NOT NULL))`. RLS can't see the old row, so a request allowed to delete can, through raw SQL, also change other columns while soft-deleting; the guide says so. The proptest found this (a locked post its writer may delete) at a larger case count.
- **Runtime.** `Db::with_row_security()` (fails on SQLite) makes every Postgres transaction begin with one `SELECT set_config($1, $2, true), …` for the bypass (`on` for `system`) and every registered setting (empty when the request has none), and wraps statements outside a transaction in one. The migrator sets the bypass in each migration's transaction. Postgres refuses an insert its `WITH CHECK` rejects before the ORM's own check runs, so under RLS an insert first checks its row as an image (`live::image_count`, the live feed's technique) and refuses with the ORM's message; any other refusal (SQLSTATE 42501, "row-level security") maps to `DbError::Forbidden`.
- **Tests.** `tests/row_security.rs` (Postgres, as a non-superuser owner, since superusers bypass RLS): the rendered policies (snapshot); raw SQL through request handles, a transaction, and a second client that sets nothing, each filtered, with refused raw updates, deletes, and inserts; migrations bypassing; and a proptest (48 cases) comparing what requests read, count, bulk-update, insert, and delete on an RLS database and a plain one. Sending settings under the wrong name, not forcing RLS, or skipping the insert pre-check each fails it; removing the migrator's bypass fails the migration test. Unit tests cover the refusals and `NULL` semantics; `leptoskit_auth`'s tables export as RLS.
- **Not built.** Upserts and saves refused by `WITH CHECK` get Postgres's message rather than the rule's; views (`security_invoker`) and the audit log under RLS; RLS for SQLite (none exists).

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

**As built (phase 5, projections):**

- **Declaring.** `projection(SelfUser, Staff)` on `#[db::table]` and `#[db(private, in(SelfUser))]` on a column. Each projection is generated exactly like the public type (serde, `JsonSchema`, `validate`, `From<Row>`, always compiled for clients): the public columns plus the private ones marked `in` it, with flattened mixins nested as their public type. A private flattened mixin field may be `in(...)` a projection; a mixin's own fields may not.
- **Refused.** `in` on a `secret` column (it never leaves the server), on a public one (already in every projection), naming an undeclared projection, and a projection declared twice or named like the public type.
- **Moved out.** Shapes checked against a projection (`#[db(from = User, projection = SelfUser)]`) rather than only `public`.

**As built (phase 4, policies and globals):**

- **Globals and `Context`.** Globals are any `Send + Sync` value in a `Context`, found by type, rather than a `#[db::global]` attribute. `cx.global(|v: &Viewer| …)` builds a condition from one and is `Expr::never()` without it.
  - `cx.user_id()` and `cx.has_role(…)` come from `leptoskit_auth`, as globals it defines.
  - `cx.count(query)` and `cx.exists(query)` count linked rows; see "As built (counts in rules)" below.
- **Access.** A `Db` carries an `Access`:
  - `Db::connect` gives an empty `Context`, so policies fail closed;
  - `with_context` scopes a handle to a request;
  - `system(reason)` bypasses policies; phase 5 records the reason, and the actor of the handle it was made from, in the audit log;
  - transactions inherit the access of the `Db` that began them.
- **Reads, updates and deletes** add the action's condition to `WHERE`: `allow`s OR-ed, AND NOT the OR of the `deny`s. A table without an `allow` for an action gets `1 = 0`.
- **Checking writes.** Inserts, saves, bulk updates and upserts run in a transaction (`atomically!`), and then count that the written rows meet the condition. Counting allowed rows, rather than searching for disallowed ones, means a rule that evaluates to `NULL` refuses instead of admitting.
  - **Upserts** first check that an existing row they'd overwrite is updatable.
  - **Explaining refusals.** A refusal names the first matching `deny`'s message, then the first `allow`'s, then a generic one.
  - **Not found versus forbidden.** A single-row write that matches nothing reports `NotFound` when the request can't see the row, `Stale` for an old version, and `Forbidden` otherwise.
- **Cascades** are integrity rather than access, so they reach linked rows whatever those tables' policies say.
- **Macro.** `#[db::policy]` sits below `#[db::table]`, which consumes it. A misplaced one is a compile error, and an imported `policy` is marked used.
- **Moved out.** RLS mode stays in phase 7, and per-audience projections are in phase 5.

**As built (counts in rules):**

- **Shape.** `cx.count(query)` returns a `Count`, compared with `eq`, `ne`, `gt`, `gte`, `lt` or `lte` to make an `Expr::Count`. `cx.exists(query)` is `count(query).gt(0)`. Both dialects render it as `(SELECT count(*) FROM <table> AS "lk_count_<depth>" WHERE …) <op> $n`, so a policy stays one statement.
- **Correlation.** `Col<U, V>` is an `Operand` for comparisons (`Operand<V>`, `Operand<Option<V>>`, and `Col<U, Option<V>>` as `Operand<V>`), which builds `Expr::CompareColumn`. A column has no value, so `set` and `in_list` with one fail with `DbError::Encode`.
- **Resolving columns.** `Sql` keeps a scope stack: the statement's table (named by `Sql::table_name`, unaliased), then each count's alias. A compared column resolves to the outermost scope of its table at or inside the condition's origin; the innermost is written unqualified. A count's own policy is written `isolated`, with the origin at the count, so its rules read the counted rows. A column of a table in no scope fails the query.
- **Same table on both sides.** In a self-count, the right-hand column is the checked row's, because it resolves to the outermost scope. Comparing two columns of the counted rows inside a self-count isn't expressible yet.
- **Policies inside counts.** A count applies the counted table's `select` policy for the same `Context`, and leaves soft-deleted rows out unless the query has `with_deleted()`. Like Gel, a table's read policy doesn't apply to counts of that table inside itself: a thread-local stack of read policies being evaluated (`Reading`) skips it, which is what makes "members see their teams' members" possible and ends cycles between tables.
- **Fail-closed.** Counts never yield `NULL`, so the write checks still count allowed rows. A correlated comparison with a `NULL` matches nothing, so `count(…).gt(0)` refuses; rules shaped `count(…).eq(0)` allow, and the docs say to wrap them in `cx.global`.
- **Tests.** `tests/policy_counts.rs` checks reads, counts, inserts with a cap, bulk updates and deletes against a plain-Rust model with proptest (96 cases on SQLite, 48 on Postgres), plus fixed cases and misuse. Mutating the count's soft-delete filter or the outer qualification fails it on both databases. The SQL itself is pinned in `policy.rs`'s unit tests.

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

**As built (phase 5, views):**

- **Model.** `Schema::views: Vec<View>` (absent from snapshots when empty, so existing `schema.json` files read unchanged), `View { name, columns, sql: Generated, materialized, comment }`, `Schema::with_views`, `Schema::view`, `View::same_on(other, dialect)`. Validation: identifiers, a flat name unique across tables and views, at least one column, no duplicate columns, no defaults or generated columns, no arrays on SQLite, a non-empty query for the dialect.
- **Diff.** `Step::DropView` (first phase) and `Step::CreateView` (last, after the final `DROP TABLE`), both `Risk::Safe`. A view is recreated when it changes on the project's dialect (query, columns, `materialized`, comment). Because a view's SQL isn't parsed, any table step other than `CreateTable`, `CreateIndex` and comments drops every previous view and creates every target view: Postgres refuses to retype or drop a column a view reads, and SQLite's `ALTER TABLE … RENAME` in a rebuild fails while a view names the table. Recreating refills materialized views.
- **Render.** Both dialects write `CREATE VIEW name (columns) AS query`, so the declared column names are the view's. Postgres: `CREATE MATERIALIZED VIEW … AS` (filled on creation), `COMMENT ON [MATERIALIZED] VIEW`, schemas for views' namespaces. SQLite has no materialized views, so `materialized` is a `STRICT` table of the declared columns (types and checks from the column model) plus `INSERT INTO … query`, and dropping it is `DROP TABLE` (decision: the plan said Postgres-only; emulating it keeps one declaration working on both).
- **Macro.** `#[db::view(name, sql | sql(postgres, sqlite), materialized, namespace, public, projection, crate)]`, expanded by the table macro with a hidden `__view` marker. No key; fields take only `column`, `private`, `secret`, `in(...)`; policies only `select`. Generates the row, `Debug`, columns, `Table` (`KEY = []`, `READ_ONLY = true`), `View` (`MATERIALIZED`, `view()`), `query()`, `refresh()` for materialized views, link accessors for `Id<T>` columns, client types, and a `ViewEntry` in `inventory`; never writes.
- **Runtime.** `schema()` adds the registered views. `refresh::<V>` is `REFRESH MATERIALIZED VIEW` on Postgres and `DELETE` + `INSERT … query` in `atomically!` on SQLite; on a plain view it's `ReadOnly`. `Update::run`, `Query::delete` and `destroy` refuse views with `DbError::ReadOnly`. Policies apply as on tables (decision: a view with none is open to server code, like a table; the docs say a view's SQL reads every row regardless of base-table policies).
- **Tests.** `tests/views.rs` (both databases): reads, filters, link accessors, public types, live data, materialized refresh (including a rolled-back refresh), refused writes, a migration that retypes a table column and recreates views, changing one view's query, dropping views, a denying view policy, and a proptest of a view's `select` policy against a model (64 cases on SQLite, 32 on Postgres; ignoring view policies fails it on both). `leptoskit_db_schema`'s scenarios `create_views`, `views_follow_table_changes` (an SQLite rebuild and a Postgres `ALTER TYPE` under views), `change_a_view`, and `drop_views` render and apply on both databases.
- **Moved out.** Indexes on materialized views (and so `REFRESH … CONCURRENTLY`), views that select from other views (creation order is by name), and shapes starting from a view. All three landed in phase 6:

**As built (phase 6, views):**

- **Views over views.** `View::may_read(name, dialect)` scans the query's identifiers (bare, quoted, and `a.b`, skipping string literals and `--` comments, case-folding bare ones) for another view's name, flat name, or `namespace.name`. False positives only order more carefully. `Schema::views_in_order` sorts by dependency, then name, and returns the cycle when there is one, which validation reports. The diff recreates, besides changed views, every view that reads a recreated one (to a fixpoint over both schemas), drops each before the views it reads, and creates each after them. Scenarios `views_over_views` and `change_a_view_another_reads` render and apply on both databases.
- **Indexed materialized views.** `View::indexes` (absent from snapshots when empty). `#[db::view(materialized, index(…), unique(…))]` builds them like a table's; the macro refuses them on a plain view, and validation refuses them on one too, plus unknown columns, duplicate names across tables and views, and empty indexes. Both renderers create them after the view (Postgres on the materialized view, SQLite on its stand-in table); changing them recreates the view. A view with `unique(…)` gets `refresh_concurrently()`: `REFRESH MATERIALIZED VIEW CONCURRENTLY` on Postgres, `refresh` on SQLite (decision: SQLite's refill transaction already lets readers carry on).
- **Shapes from views** needed no change: a view is a `Table` with columns, public columns, and `__via_*` relations for its `Id<T>` columns. Nested relations _to_ a view still can't order (no key). `tests/views.rs` checks it with the view's policy on both databases.

### 4.11 Annotations, modules, branches, extensions

| Gel                  | leptoskit_db                                                                                                                                                                                                                                     |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Annotations          | Doc comments go to `COMMENT ON` (Postgres), to `schema.json`, to JSON Schema descriptions (and so to the generated client docs), and to the admin studio. `#[db(deprecated = "use display_name")]` warns at compile time on use.                 |
| Modules              | Rust modules organise the code. `#[db::table(schema = "billing")]` maps to a Postgres schema, or to a `billing_` table prefix on SQLite.                                                                                                         |
| Branches             | **A dev database per git branch.** `lk dev` uses `myapp_dev_<branch>` (Postgres, cloned from `main`'s dev database with `CREATE DATABASE … TEMPLATE`) or `.leptoskit/db/dev-<branch>.sqlite`. Switching branches no longer breaks your dev data. |
| Extensions           | **Schema packs** (§7): `leptoskit_auth`, `db::audit`, `db::search`, `db::storage`, each a crate or feature that contributes tables to your migrations.                                                                                           |
| Introspection and UI | `schema.json` is the introspection format. `lk db studio` serves a local data browser, itself built with leptoskit, that reads through the app's own policies: `secret` columns never show, `private` ones do (no `--reveal` yet).               |
| Future behaviours    | `[db] edition = 1` in `leptoskit.toml`. Behaviour changes ship behind a new edition, and `leptoskit migrate` upgrades it.                                                                                                                        |

**As built (phase 7, refusals under row-level security):** a save, or an upsert, under `Db::with_row_security()` checks the row it would leave against its rules before writing it (`write::refuse_result_before_row_security`, through `policy::refuse_image`), as inserts already did, so a refused write reports the rule that refused it rather than Postgres's "new row violates row-level security policy". The row a save leaves includes its incremented version; the row an upsert that overwrites leaves keeps the old row's key, target, `created_*`, generated, and slug columns, as `DO UPDATE` does. `tests/row_security.rs` shows the refusals are the same with RLS on and off, and differ without the check.

**As built (phase 7, the studio):**

- **Where it runs.** Inside the app's db crate, so that every read goes through the app's compiled policies: `lk db studio` runs the crate's `export_schema!()` test (`cargo test --lib -- __leptoskit_db_export_schema --nocapture`) with `LEPTOSKIT_DB_STUDIO` set to the address, plus the database URL, the migrations directory, the dialect and the policies mode, and with `leptoskit_db/studio` (or `leptoskit/db-studio`, whichever the crate depends on) and the dialect's driver added to `[db] features`. `export_schema_if_requested` serves instead of exporting. `lk dev --studio` spawns the same command beside the app and kills it when `dev` stops.
- **Out of release builds.** `leptoskit_db::studio` and every field and function that reaches it are `cfg(all(feature = "studio", debug_assertions, any(postgres, sqlite)))`; without them `export_schema_if_requested` panics with why. `tests/studio_release.rs` builds `tests/studio_fixture` with the feature on in release (its own target directory, since the running test holds the main one's lock) and checks that the studio's title and style sheet aren't in the binary, while the debug test binary, the positive control, holds both.
- **Reaching it.** Binds a loopback address only (anything else is an error). Each run makes a 32-byte token with `getrandom`; a middleware compares it in constant time from the query, requires `Host` to be the bound address (DNS rebinding), refuses a foreign `Origin`, and sets `no-store`, `no-referrer` (the token is in the URL), `DENY` framing, `nosniff`, and a CSP with `default-src 'none'`.
- **Tables, generically.** `TableEntry` carries `studio::TableOps`, function pointers instantiated per table: reads build a `Query<T>` from a filter (`=`, `≠`, `>`, `<`, contains, null) parsed per column type, order by any column, and page by 50; writes find the row by its key, replace one column's value, and call `write::save` or `write::delete`, so hooks, validation, versioning, policies, and the audit log all apply. A write holds the row across awaits, and rows needn't be `Send`, so it runs on a blocking thread with `Handle::block_on`. `secret` columns always read as `‹secret›`.
- **Viewers.** Anonymous, `system` (a typed reason is required; each distinct reason is leaked once to get the `'static` `Db::system` takes, and recorded as `studio: <reason>`), or a `Persona` registered through `inventory` with `register_persona!`: a name, a description, and `fn(&str) -> Result<Context, String>`. `leptoskit_auth` registers `account` (one factor, a nil session). The viewer travels in every link and form, and a `POST` whose form names another viewer than its address is refused.
- **Edits.** Only with `edit=1`, in both the address and the form; a first submit renders a confirmation, and only a second with `confirmed=yes` writes.
- **UI.** leptos views rendered on the server with `to_html()`, as `leptoskit_docs` does, with no hydration and no script: the studio runs under `cargo test`, which has no wasm toolchain or bundler in its path. The live page follows `db.changes()` per viewer (16 viewers, 200 changes each) and reloads itself every two seconds.
- **Moved out.** Inserting rows, views' rows, `--reveal` for `private` columns (they show; `secret` ones never do), and following a branch switch under `lk dev --studio` without a restart.

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

**As built (phase 7, full-text search):**

- **Declaring.** `#[db(search)]` or `#[db(search(weight = A|B|C|D))]` (default `D`, Postgres's) on text columns of a table, a link, or a mixin's fields; `#[db::table(search(language = "…"))]`, default `simple`. The macro implements `Searchable` (`LANGUAGE`, `SEARCHED: &[(column, SearchWeight)]`), asserts each column is `TextColumn` (now with an `on_unimplemented` message; `Email`, `Url`, `Slug` qualify), and refuses `search` on views, generated columns, and flattened fields, and a language with nothing searched. A trybuild case pins searching an `i64`.
- **Model.** `Table::search: Option<Search { language, columns: [SearchColumn { name, weight }] }>`, absent from snapshots when `None`. Validation: lowercase-snake language; on SQLite only `simple` or `english` (others need Postgres, so a pack is portable only with those); at least one column, each existing, text, not generated, once; no column named `lk_search`; no key column named `lk_row`; companion table names and the GIN index name unique.
- **Postgres.** A stored generated `lk_search tsvector` (`setweight(to_tsvector('<language>'::regconfig, coalesce(col, '')), 'A') || …`; a constant configuration keeps `to_tsvector` immutable) with `<table>_lk_search_idx` `USING gin`. Queries never select it.
- **SQLite (decision: not external content).** External-content FTS5 keys entries by the table's implicit `rowid`, which `VACUUM` and the migrator's table rebuilds renumber. Instead, `<table>_lk_search_rows (lk_row INTEGER PRIMARY KEY, <key columns>, UNIQUE(key))` numbers rows by key, and a contentful `<table>_lk_search` FTS5 table (columns `c0…`, since FTS5 reserves names such as `rank`; tokenizer `unicode61 remove_diacritics 0`, `porter` first for `english`) holds the text with `rowid = lk_row`. Triggers after insert, after update of the searched and key columns, and after delete keep both current, so raw SQL writes are indexed too. Contentful costs a copy of the text but makes `highlight` and `snippet` work without a join back.
- **Diff.** `Step::DropSearch` (first phase, with views) and `Step::CreateSearch` (after foreign keys, before comments), both `Risk::Safe`. An index is recreated when it changes, when its table is renamed, when a searched or key column is renamed, retyped, or dropped, or when the key changes; creating one backfills existing rows. Postgres skips `DropSearch` for a dropped table. A SQLite table rebuild drops the old table's triggers, so the rebuild re-creates them unless the plan recreates the index; the companion tables survive, being keyed by key. Scenarios `add_search`, `rebuild_a_searched_table`, `change_search`, and `drop_search` render and apply on both databases, and the apply tests then insert, retitle, and delete posts and count matches.
- **Querying.** `Query::search(text) -> Search<T>` with `filter`, `order_by` (ties after rank, then the key), `limit`, `offset`, `with_deleted`, `highlight(col)`, `excerpt(col, words)`, then `all`, `first`, `hits` (`Hit { row, rank, highlights }`), and `count`. The text is parsed in Rust (words AND-ed, `"phrases"`, `-excluded`, `OR`, `word*` prefixes for words of letters, digits, and `_`), dropping terms without a letter or digit and groups with nothing required, so a search is never only exclusions; at most 32 terms. Postgres gets `phraseto_tsquery`/`to_tsquery(…':*')` per term, each a bound value, combined with `&&`, `!!`, `||`, matched with `lk_search @@ …` and ranked with `ts_rank`; SQLite gets one bound FTS5 expression of quoted strings (`(("a" AND "b"*) NOT "c") OR …`), so nothing the user types is syntax, joined to the table through the rows table, ranked with `-bm25` weighted by `SearchWeight::factor` (Postgres's `ts_rank` defaults). Both then add the query's filter, the `select` policy, and the soft-delete filter through `query::write_conditions`.
- **Highlights.** Marked with U+E000/U+E001 in the database (`ts_headline` with `HighlightAll` or `MaxWords`/`MinWords`/`MaxFragments=1`; FTS5 `highlight`/`snippet`), split into `Highlighted { segments: [Segment { text, matched }] }` in Rust, so the API never returns HTML. Both dialects strip those two characters from indexed and highlighted text, so content can't forge a marker.
- **Known differences.** Postgres's `english` drops stopwords and SQLite's `porter` keeps them; excerpts choose differently; rank values differ (order agrees in the tested cases).
- **Tests.** `leptoskit_db/tests/search.rs` (both databases): ranking by weight, the syntax, punctuation and injection-looking input, highlights and excerpts, private-use characters, misuse, every write path (save, bulk update, upsert, soft delete, destroy, key moves, raw SQL), composite keys and namespaces, and policies by viewer. A proptest of what each viewer finds over random articles (writers, published, deleted, title and body words) and random queries (words, exclusions, `OR`) against a plain-Rust model, also checking ranks descend and `count` agrees (96 cases on SQLite, 48 on Postgres). Dropping the policy from searches, or ignoring soft deletes, fails it on both.
- **Not built.** Searching from a shape, hybrid search (with vectors, phase 8), and per-column language.

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

**As built (phase 5, shapes):**

- **Derive.** `#[derive(db::Shape)]` with `#[db(from = Table[, public])]`. A field reads a column (its name, or `column = handle`), `count(link)` with an optional `filter`, or `via = link` as another shape: `Vec<S>` for all rows, `Option<S>` for the first, and `S` for a required link's row. `filter` and `order_by` are closures over the linked table's columns, the same expressions as `Query::filter` and `order_by`, rather than the sketch's raw SQL and `created_at desc`, so they're typed and dialect-free; `limit` and `offset` take integers. `flatten_first` became `Option<S>`.
- **How the derive sees the table.** It can't read the table, so it names what `#[db::table]` generated and lets the compiler check it: `Selected::column::<Table, FieldType>(Table::columns().field)` fails on a wrong type or name; a `public` shape uses `Table::__public_columns()` instead, a generated `{Table}PublicColumns` with no private or secret columns, and asserts each nested shape is `PublicShape`. `via = posts` calls `Table::__via_posts()`, a `Relation<From, To, One | MaybeOne | Many>` that the table generates for each link (forward), `backlink` (backward), and `#[db::link]` end (through the link table). The field's type picks `one`, `optional`, or `many`, which exist only for the right cardinality. Trybuild cases in the pack fixture pin the errors.
- **SQL.** One statement of correlated scalar subqueries in the select list, the same on both dialects, rather than `LEFT JOIN LATERAL`, which SQLite lacks: `(SELECT count(*) …)`, `(SELECT json_build_array(…) … LIMIT 1)`, and `(SELECT coalesce(json_agg(lk_value ORDER BY lk_rank), '[]') FROM (SELECT json_build_array(…) AS lk_value, row_number() OVER (ORDER BY …) AS lk_rank … LIMIT n))`, with `json_array`/`json_group_array` on SQLite. Every level opens its own `lk_shape_<depth>` scope, correlates by explicit aliases (so a tree three levels deep correlates each level with its own parent), and writes the target's `select` policy `isolated` in that scope plus its soft-delete filter; a link table adds its own policy and soft deletes in an `IN (SELECT far FROM link AS lk_link_<depth> …)`. Nested orders end with the target's key, so results are deterministic. SQL for both dialects is pinned by insta snapshots in `tests/snapshots/shapes__*`.
- **Decoding.** Top-level columns decode as ever. Nested rows are positional JSON arrays (no field names to collide or escape), decoded by each field's `ColumnType`: SQLite UUIDs and bytes are hex (`CASE … lower(hex(…))`, since `hex(NULL)` is `''`), Postgres bytes `encode(…, 'hex')`, SQLite JSON columns `json(…)`, and Postgres's text floats (`NaN`) parse back. SQLite loses JSON's subtype through subquery columns, so nested values are re-parsed with `json(…)`. A null required row is `DbError::NotFound`, as the link accessor is. `Decimal` and array columns can't be nested yet. Shapes hold at most 100 fields, both databases' function-argument limit.
- **Policies.** Each nested table's `select` policy and the link tables' apply for the request's context, and `Db::system` bypasses them all, as for queries. `tests/shapes.rs` (both databases) checks counts, ordering, limits, optional and required links, every column type nested, a three-level self-referencing tree, soft deletes, bypassing, and a proptest of nested posts and tags per viewer against a plain-Rust model (96/48 cases); dropping the nested policy or the link table's fails it on both.
- **Moved out.** `with_deleted` for nested rows (phase 6, below); a link's own properties inside a shape (Gel's `@position`), which needs a relation from each end to the link rows themselves; and starting a shape from `Links`.

**As built (phase 6, shapes):** `#[db(…, with_deleted)]` on a `via` or `count` field (any nesting, including a required link, where it reads a soft-deleted row instead of `NotFound`) calls `Related::with_deleted`, which leaves the target's soft-delete filter out; a link table's own soft-deleted rows still link nothing. Refused on a column and when given twice. Link properties inside shapes are still left: the relation would have to nest the link row and the far row as one value, and the derive would need a second shape type per field; `Links::all` covers reading them meanwhile.

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

**As built (phase 3 leftovers: dev Postgres and the dev watch):**

- **Config.** `[db] dev_server = "auto" | "devenv" | "embedded" | "off"` (default `auto`) and `watch = "migrate" | "report" | "off"` (default `migrate`). Both are left out of `leptoskit.toml` while they're the defaults, so the scaffold's file didn't change. `lk new --db` with no value is Postgres.
- **Choosing the database.** `db.url_env` set wins, and nothing is started. Otherwise, for Postgres, `auto` picks devenv when `devenv.nix` has an uncommented `services.postgres.enable` or `services.postgres = …`, and the embedded server otherwise. SQLite never gets a server.
- **devenv.** Reads `PGHOST` (socket directory or host), `PGPORT` and `PGUSER` from `devenv shell`, and fails with a hint when they're missing or the socket isn't there. It never runs `devenv up` itself.
- **Embedded.** `postgresql_embedded` 0.22 (`blocking`, `theseus`, `tls-rustls-ring`; no `bundled`), Postgres `=18`, downloaded once into `$XDG_CACHE_HOME` or `~/.cache`, under `leptoskit/postgresql`. Per project, `.leptoskit/db/postgres` holds `data/`, a `password` and a `port`, both made on the first run with mode 0600. A server already running, found by `postmaster.pid` plus a TCP connect, is reused and never stopped by the second user; one this process started stops on drop. `LEPTOSKIT_POSTGRES_BIN` runs installed binaries instead (`trust_installation_dir`), which is also how the tests avoid the download.
- **Database.** Each source's URL names `postgres`; the app's database, `app.name` with `_` for `-`, is created when missing, and the URL then points at it.
- **Where it's used.** `lk dev` starts it before the first build and passes `url_env=<url>` to the app server; it outlives the server. `lk db migrate`, `status` and `reset` use it when neither `--url` nor `url_env` is given.
- **The dev watch.** `lk dev` flags changes under the db package's directory, then, before rebuilding, exports the schema and diffs it against the latest snapshot with `RequireHints`. Additive changes (no `LosesData` step) go through the normal `create` path, named after the first step, and are applied with `db.migrate` when `lk dev` knows the URL; a migration that fails to apply is deleted again with its registry line. Destructive changes and diff errors write nothing and print the command to run. It also runs once at startup.
- **Left for phase 6 (push mode).** Each additive change is its own migration; amending one working migration, or pushing without migrations and reconciling when one is created, needs bookkeeping in `_leptoskit_migrations` that belongs with push mode. Changes to tables in pack crates aren't watched, and the embedded server has no PostGIS (§4.13).

**As built (phase 6, push mode):**

- **Runtime.** `_leptoskit_push` (`id = 1` only, `schema`, `pushed_at`), created on the first push. Hidden `Db::push(migration, schema_json)` runs SQL like a migration (its own transaction, the Postgres advisory lock, SQLite's foreign-keys-off directive and check) and upserts the row instead of recording a migration; `Db::pushed_schema()`; `Db::adopt(migration)` records a migration as applied without running it and deletes the row in one transaction, failing with `MigrationError::NothingPushed` without one. `Db::migrate` fails with `MigrationError::Pushed { name }` when the database has pushes and a migration it hasn't had would apply (decision: a teammate's migration never lands on top of half-finished changes; nothing pending is fine, so the server starts). Both drivers share `migrate::Record { Applied, Pushed, Adopted }`.
- **CLI.** `[db] watch = "push"`. The dev watch diffs what the database holds (the pushed schema, or the latest snapshot) against the export with `RequireHints`, applies pending migrations first, and pushes; destructive plans push nothing and point at `leptoskit db push --allow-destructive`; pushing back to the latest snapshot clears the row. `leptoskit db migration create` in push mode resolves the development database first and, when it has pushes, refuses before writing anything if catching it up deletes data without `--allow-destructive`; then writes the migration (latest snapshot → tables, the usual diff), pushes the remainder, and adopts; a failure removes the migration again. `leptoskit db status` reports pushes. Tests in `db/tests.rs` and `db/watch.rs` run on SQLite and the embedded Postgres.
- **Never loses data silently:** destructive pushes need the flag; adoption never runs the squashed SQL over the pushed tables; migrations refuse to stack on pushes.

**As built (phase 6, branch databases):**

- **Naming.** `[db] branch_databases` (default `true`, left out of `leptoskit.toml` while default). The main branch is `origin/HEAD`'s, else `main`, else `master`, else the current one; it, a detached `HEAD`, and projects outside git keep `app.name` with `_` for `-`. Other branches use `<app>__<branch>` (lowercase letters, digits, `_`), cut to 63 bytes with eight hex digits of the branch name's SHA-256.
- **Creating.** `dev_server::start` now calls `branches::ensure`: the main database is created if missing, and a branch's with `CREATE DATABASE … TEMPLATE <main>` plus `COMMENT ON DATABASE … IS 'leptoskit branch: <branch>'`. Postgres refuses to copy a database something is connected to, which gets a message naming the cause. Only the development Postgres (devenv or embedded) gets branch databases; `url_env` and SQLite are left alone (decision: never override a database the user named).
- **Switching.** `leptoskit dev` watches the directory of `git rev-parse --git-path HEAD`; on a change it stops the app server (so the copy can run), calls `DevDatabase::switch_branch`, re-syncs the tables, and the rebuild restarts the server with the new URL. `leptoskit db migrate|status|push|reset|migration create` use `Project::database`, which goes through the same `start`.
- **Cleaning up.** `leptoskit db branch list` reads `pg_database` names with the app's prefix and their comments; `prune` keeps the checked-out and main branches and deletes (only with `--yes`, `DROP DATABASE … WITH (FORCE)`) those whose branch is gone or `git branch --merged <main>`.
- **Tests** (`db/branches.rs`, `db/watch.rs`) on the embedded Postgres with throwaway git repositories: copying with rows, switching back and forth, the busy-template error, listing, pruning deleted and merged branches but not the current or unmerged ones.

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
  - link only within the pack, or to another pack's tables (as `idp` links to `auth.accounts`), never to an app's;
  - avoid raw SQL defaults and checks.
- **Moved out.**
  - Bulk `Query::delete`/`destroy` on a pack table aren't restricted yet, since they hang off the shared `Query` type. Packs should guard them with policies.
  - Packs-as-values (`leptoskit_auth::pack()` carrying server functions and hooks) waits for `leptoskit_auth`, which will show what a pack needs beyond tables.

## 8. Live data

[realtime-plan.md](realtime-plan.md) expands this section into a full design: live queries, change capture in migrations, fan-out, and deployment.

This is unchanged from revision 1: invalidate, don't replicate. Policies make it safer still, because a refetch runs the same policy-filtered query. Revision 2 adds:

- **Policy-aware event filtering:** an event only goes to subscribers whose policies could see the old _or_ the new row, so the existence of hidden rows doesn't leak.
- **`LISTEN/NOTIFY` fan-out on Postgres**, for running more than one server.

**As built (phase 6):**

- **Opting in.** `#[db::table(live)]` sets `Table::LIVE`; links take it too, views refuse it. `audit::records::<T>()` (`AUDIT || LIVE`) replaced every `T::AUDIT` gate, so a live table's writes take the audit log's paths: single-row writes in `atomically!` with the stored rows, bulk updates and soft deletes reading and locking rows, bulk destroys with `RETURNING`, soft cascades, and the database's own cascades (§4.8, `fallout`). `Subject` gained `audit` and `live`; `record_changes` writes audit entries for one and `live::RowChange`s for the other (normalized images in column order, `secret` columns `NULL`, the key).
- **Publishing on commit.** `Connection::publish` (hidden): a `Tx` buffers changes and publishes on `commit`, never on rollback or drop. On SQLite, after `COMMIT`, into the pool's `Hub` (a `tokio::sync::broadcast` of 1024 change sets). On Postgres, before `COMMIT`, as `SELECT pg_notify('leptoskit_live', …)` inside the transaction, which Postgres delivers only if it commits. A `Db` publishing outside a transaction (no path does today) notifies at once.
- **The wire.** One JSON array per transaction with every value tagged by kind (floats as text, so `NaN` survives), split into parts below 7000 bytes on character boundaries, each `<uuidv7>:<index>:<count>:<part>`; `Assembler` joins them and refuses parts out of order. Secrets never travel, since images hold none.
- **Listening.** The first `Db::changes()` on Postgres starts `postgres::Listener` (a `tokio_postgres` connection of its own, with the pool's TLS choice, polled with `poll_message`) and returns once `LISTEN` took effect, so a subscriber hears everything committed after it subscribed. A dropped connection reconnects with backoff (100 ms to 30 s) and sends `Event::Lost`; the task stops when the last handle drops the hub. Every server, including the writer, hears changes through it.
- **Filtering.** `Feed::next` groups a change set by table and, per table, builds statements from the subscriber's access with `live::visible_images::<T>`: `SELECT lk_live_image FROM (SELECT 0 AS lk_live_image, CAST($1 AS uuid) AS "id", … UNION ALL …) AS "lk_live" WHERE <select policy>`, with Postgres casts from `leptoskit_db_schema::postgres_type`, and `Sql::root_alias` so a policy's correlated counts name the images. A change is delivered when either image passes; soft-deleted images count (decision: readers may ask for them with `with_deleted`). `System` access and tables without policies skip the query. A table this build doesn't know, a lagging subscriber, and a lost connection yield `Invalidation::Everything`.
- **Client.** `leptoskit_db::live::{Invalidation, ChangedRows}` are client-safe serde types (`{"rows":[{"table","keys"}]}` or `"everything"`). `leptoskit_query`'s new `db` feature adds `live::{table_tag, row_tag}` (`["db", table]`, `["db", table, key…]`), `QueryFilter::tags_where`, and `QueryClient::apply_invalidation`, which invalidates queries tagged exactly with a changed table or under a changed row, or every `["db"]` query for `Everything`. The facade's `db` feature turns it on with `query`.
- **Not built.** A transport: apps send invalidations over their own SSE or WebSocket route (a built-in `/db/live` route and a `use_live_invalidations` hook would be next). Raw SQL, migrations, and materialized view refreshes publish nothing. Per-subscriber visibility costs one query per table per change set.
- **Tests.** `tests/live.rs` (both databases): every write path, one set per transaction, rollbacks and dropped transactions, before/after visibility for owners, strangers and signed-out requests, secret columns failing closed, cascades, two servers on one Postgres with a change set split across notifications, and lagging. A proptest of what four viewers hear over random histories (inserts, saves, ownership changes, soft and hard deletes, bulk updates) against a plain-Rust model (96 cases on SQLite, 48 on Postgres); checking only the after-image, or ignoring the policy, fails it on both. `leptoskit_query`'s `tests/live.rs` checks which queries go stale.

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
5. **Shapes, multi links, hooks and audit, views, generated columns, projections.** Multi links, shapes, projections, and generated columns landed first; hooks, `created_by`/`updated_by`, slugs, the audit log, and views followed (see each section's "As built (phase 5 …)"). Phase 6 added replacing a link set, `with_deleted` for nested rows, auditing database-level cascades, indexed materialized views with `refresh_concurrently`, views over views, and shapes from views. Still left: `on_source_delete = delete_orphan_targets` and link properties in shapes; the `links(tags: Tag)` shorthand is declined (§4.2).
6. **Branch databases, push-mode dev loop, live invalidation.** Built (§4.11 via §6's "As built (phase 6 …)", §8). Left: a built-in transport for live invalidations, and publishing raw SQL and view refreshes.
7. **Polymorphic tables, RLS mode, full-text search, studio.** Built: full-text search (§4.12), RLS mode (§4.9), polymorphic tables (§4.6) and the studio (§4.11); see their "As built (phase 7 …)" notes. Still left: RLS for views (`security_invoker`) and for the audit log, whose reader is a Rust global rather than a table; searching from a shape; shapes over a polymorphic enum, and ordering through it by anything but the key; and in the studio, inserting rows, `--reveal`, and following a branch switch under `lk dev --studio` without a restart.
8. **Geospatial and vectors** (tier 4), in two stacked PRs. Geospatial is built (§4.13, "As built (phase 8, geospatial)"); its "Not built" list is what's left of it.
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
