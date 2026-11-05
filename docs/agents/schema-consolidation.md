# The schema, measured against Gel

Written 5 November 2026, against `main` at `6e5e288dc`. An investigation of two questions:

1. **Expressiveness.** How does `leptoskit_db`'s schema and query surface compare with [Gel Data](https://geldata.com) (formerly EdgeDB), which is the reference for expressive declarative models? What is missing, what is ahead, and what is worth adding?
2. **Migrations.** What stands between today's migration system and "seamless"? In particular: release-aware consolidation, so a migrations folder holds one migration per release instead of thousands, the way [Pina](https://github.com/pina-rs) pins published history to deployments and the way monochange defines when a release happens.

Everything in §2–§3 is descriptive of the code as it is; §7 and §8.4 onward are proposals.

---

## 1. Why Gel, and what Gel is now

Gel is the right thing to measure against: it is the most expressive schema-as-code system shipped, and its query language (EdgeQL) is designed around object paths rather than joins. Two facts change how we should borrow from it:

- **The company is gone.** Gel Data announced on 2 December 2025 that it is winding down and joining Vercel; Gel Cloud stopped accepting registrations immediately and shut down 31 January 2026. The database remains open source ([geldata/gel](https://github.com/geldata/gel)), with Gel 7.0 released 5 November 2026. There is no vendor left to converge on, so the ideas are free and the product is not a competitor.
- **Gel is not Convex.** Convex infers migrations from model declarations with no migration files at all. Gel, like us, has an explicit `migration create` step, interactive prompts, and a committed migrations directory; its "declare and it manages" feel comes from `gel watch --migrate` dev mode plus a server-side differ. Our `leptoskit dev` with `watch = "migrate"` is already the same loop, and §8.4's amend-unreleased mode closes the remaining distance — arguably past Gel, whose migrations always stack.

What follows is the full inventory of both sides.

---

## 2. Gel Data's feature list

### 2.1 The model (SDL, in `*.gel` files)

**Types and composition**

| Feature                  | Notes                                                                                                                                                |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Object types             | `type Person { … }`; every object gets a required readonly globally-unique `id: uuid` automatically. No primary keys to declare.                     |
| **Multiple inheritance** | `type Student extending Person, HasName`; inheritance is structural; shared pointers merge when compatible. Not single inheritance.                  |
| Abstract types as mixins | `abstract type HasName { … }` is the reuse mechanism; abstract properties, links, constraints, and annotations also exist standalone.                |
| Overloading              | `overloaded name: str` in a subtype can only narrow (tighter type, more constraints), never generalize.                                              |
| Aliases                  | `alias Published := (select Post filter .published);` — named, introspectable, non-materialized expressions. **No materialized views exist in Gel.** |

**Scalars** (exact names): `int16 int32 int64`, `float32 float64`, `bigint decimal`, `str bool json uuid bytes`, `sequence` (shared auto-increment counter), `datetime duration`, `cal::local_datetime cal::local_date cal::local_time cal::relative_duration cal::date_duration`. Enums are `scalar type Color extending enum<Red, Green, Blue>`. Collections: `array<T>` (no nesting except in tuples), named and positional `tuple<…>`, `range<T>` over numerics/datetimes, and multiranges.

**Properties, links, computeds**

| Feature                      | Notes                                                                                                                                                                 |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Cardinality on every pointer | `required`/`optional` × `single`/`multi`, independently. `multi` properties are stored as (owner, value) pairs, indexable and constrainable per element.              |
| Link properties              | Declared inside the link body (`multi family_members: Person { relationship: str; }`), addressed with `@relationship`. Required link properties landed in Gel 7.      |
| **Computed pointers**        | `full_name := .first ++ ' ' ++ .last` — expression-backed, not stored, may **traverse links**, appear in queries and indexes as if real. Volatile expressions banned. |
| Deletion policies            | `on target delete`: `restrict` (default), `delete source`, `allow`, `deferred restrict`; `on source delete`: `allow`, `delete target`, `delete target if orphan`.     |

**Constraints**: built-ins `expression`, `exclusive`, `one_of`, `min_value`, `max_value`, `min_ex_value`, `max_ex_value`, `min_len_value`, `max_len_value`, `regexp`. Object-level constraints via `on (…) { constraint expression on (__subject__ > 0) }`; **partial constraints via `except (…)`**; custom parameterized abstract constraints with templated `errmessage`; `delegated constraint` for per-subtype exclusivity pools; constraints on link properties with `@source`/`@target`.

**Indexes**: `index on (expr)` by default, `except (expr)` for **partial indexes**, tuple expressions for composites. Postgres-native kinds via `pg::hash btree gin gist spgist brin`. Special-purpose: `fts::index` (full-text), `ext::pgvector::ivfflat_*` / `hnsw_*` (vector), `ext::ai::index` (auto-embedding). Gel 7 added `build_concurrently := true`.

**Behavior in the schema**

| Feature           | Notes                                                                                                                                                                                                                           |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Access policies   | Per type: `access policy name when (…) allow select, update using (…) { errmessage := … }`. Default-deny once any exists; allows union; **deny subtracts**; inherited by subtypes; reads silently filter, blocked writes raise. |
| Globals           | `global current_user_id: uuid;` — typed session variables usable in policies, computeds, triggers; **computed globals** (`global current_user := (select User filter …)`); set per session or per client call.                  |
| Triggers          | `trigger log after insert, update for each do ( … )` with `__new__`/`__old__`, `when` guards; run in the triggering transaction; may not modify the triggering object.                                                          |
| Mutation rewrites | `rewrite insert, update using (datetime_of_statement())` on a pointer — assign-on-write intercepts with `__subject__`, `__specified__`, `__old__`.                                                                              |
| Functions         | Schema-defined EdgeQL functions, variadic/named-only params, overloading, four volatility levels; DML allowed inside since Gel 6 (call cardinality must be `One`).                                                              |
| Annotations       | `title`, `description`, `deprecated` + custom `abstract annotation`, `inheritable`.                                                                                                                                             |
| Full-text search  | Built-in `fts` module: one `fts::index` per type, `fts::with_options(text, language, weight_category)` per field, `fts::search(Object, query)` returning `(object, score)`. 22+ languages.                                      |
| Vectors / AI      | `ext::pgvector` (`vector<N>`, `halfvec`, `sparsevec`; ivfflat + hnsw); `ext::ai` generates embeddings inside the database from a declared provider and `ext::ai::search` accepts plain strings since Gel 7.                     |
| Extensions        | `using extension <name>;` → `ext::<name>`: `auth`, `ai`, `pgvector`, `pg_trgm`, `pg_unaccent`, `pgcrypto`, `graphql`, `edgeql_http`; standalone `postgis`.                                                                      |

### 2.2 The query language (EdgeQL)

Shapes are the centerpiece: nested selection sets with inline computeds, splats (`*`, `**`), link-property projection (`@name`), and polymorphic sub-shapes (`[is Movie] { … }`).

```edgeql
select User {
	name,
	posts: { title, comments: { body } } order by .created desc limit 3,
	post_count := count(.posts),
}
filter .name like 'Alice%'
order by .karma desc empty last
offset 10 limit 20;
```

| Capability      | EdgeQL                                                                                                                                                                                                             |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Paths           | Forward `.author.name`, backward `.<author[is Post]`, link props `.friends@nickname`. Deep paths compile to joins.                                                                                                 |
| Sets, not nulls | Absence is `{}`; `??`, `if … then … else …`, `exists`, `assert_single/assert_exists/assert_distinct`, `distinct`, `union/intersect/except`.                                                                        |
| Aggregation     | `count sum min max mean stddev var`, `array_agg`, `any/all`; `group … by ROLLUP(…)/CUBE(…)` emitting `{key, grouping, elements}`. **No window functions.**                                                         |
| DML             | Nested atomic inserts; upserts `insert … unless conflict on .email else (update …)`; `update … set { tags += {'x'} }` (and `-=`) on multi pointers; `for x in … union (insert …)`; `detached` for self-references. |
| Polymorphism    | `[is Movie]` shapes, `is Movie` tests, `typeof`, `<Movie><uuid>'…'` object casts; the schema itself is queryable (`schema::Migration`, `describe schema as sdl`).                                                  |
| JSON            | `<json>` casts both ways, `json_get` with `??`, `json_set`, packing/unpacking, merge `++`.                                                                                                                         |
| Misc            | `with` aliases and module aliases, parameters `$name`, string interpolation `'\(expr)'` (Gel 6), `std::net` async HTTP from queries.                                                                               |

### 2.3 Migrations

- **The server diffs, not the CLI.** `gel migration create` ships SDL to the connected instance, which diffs it against the live schema and generates EdgeQL DDL. Language-agnostic by design; requires a running database.
- Migration files are `dbschema/migrations/00001.edgeql`, each one `CREATE MIGRATION <hash> ONTO <hash> { …DDL… };` — content-addressed; hand edits force a rehash via `gel migration edit`.
- **Renames**: interactive confidence-scored prompts ("did you rename property 'name' to 'nam'?"), keys `y n l c b s q` (back, split into multiple migrations, quit). Non-interactive refuses below confidence 0.99999 without `--allow-unsafe`. **There is no rename annotation in the schema**; declining yields drop+create.
- Destructive changes prompt interactively; non-interactive creation only proceeds if everything is safe, unless `--allow-unsafe`.
- **Data migrations are first-class**: `SET REQUIRED USING ('No content')` prompts for a backfill expression that becomes DDL; `--allow-empty` stubs get hand-filled with `insert`/`update`; the documented pattern is schema step → data step → schema step across multiple migrations.
- `--squash` collapses all migrations into one (DML discarded, a `fixups/` file generated to upgrade already-migrated instances). `gel migration edit` revises the last _unapplied_ migration. `extract` rebuilds files from server history; `upgrade-check` re-checks the schema against a different server version.
- Dev mode (`gel watch --migrate`, `--dev-mode`) reconciles drift back onto recorded history. Branches (`gel branch create/rebase/merge`) rebase migration history git-style. Applied history lives in the database as introspectable `schema::Migration` objects. Bare DDL is blocked once migrations start.

---

## 3. leptoskit's feature list

Three layers: `leptoskit_db_macros` (what you write), `leptoskit_db_schema` (the `Schema` model, snapshots, diff, renderers), `leptoskit_db` (runtime). One project targets exactly one dialect, `postgres` or `sqlite`, from the same code.

### 3.1 The model

**Scalars** (`ColumnType`): `Bool`, `Int16/Int32/Int64`, `Float32/Float64`, `Decimal{precision, scale}` (schema-level), `Text{max_len}`, `Bytes`, `Uuid`, `Timestamp` (`DateTime<Utc>`), `Date`, `Json<T>`, `Array` (Postgres-only, schema-level), `Geometry`/`Geography` (PostGIS; GeoPackage WKB on SQLite), `Vector` (`Vector<N>`, `HalfVector<N>`, `BitVector<N>`, `SparseVector<N>`; 16,000 dims Postgres, 8,192 SQLite, 10⁶ sparse).

**User-defined scalars and enums**

```rust
#[derive(db::Enum, Clone, Debug, PartialEq)]
pub enum Status {
	Draft,
	Published,
}

#[derive(db::Scalar, Clone, Debug, PartialEq, Eq, Hash)]
#[db(min_len = 4, max_len = 20, regex = "^[a-z0-9_]+$", normalize(trim, lowercase))]
pub struct Handle(String);
```

`#[derive(db::Scalar)]` rules (`min_len`, `max_len`, `min`, `max`, `regex`, `normalize(trim, lowercase, uppercase)`, `validate_with = fn`) become database checks, Rust `validate()`, and JSON Schema from one declaration. Built-in: `Email`, `Url`, `Slug`, and (with the `solana` feature) `Pubkey`, `Signature`.

**Tables**: `#[db::table(name = "users", public, search(language = "english"), audit, live, …)]` with `namespace`, `projection(Type)`, `index(cols…)/unique(cols…)/check(name, sql)`, `hooks(…)`, `extends = Actor` (polymorphic), `renamed_from`, `pack`, `on_account_merge`. Column attributes: `key`, `column`, `unique` / `unique(except = deleted_at)` (partial), `index`, `private`, `secret`, `public` (vectors), `default` / `default(sql = …)`, `min/min_len/max/max_len/one_of/regex`, `created_at/updated_at/deleted_at/created_by/updated_by/version`, `slug(from = title)`, `search(weight = A..D)`, `geo(index, valid)`, `vector(distance = …, index = hnsw, source(title, body))`, `on_delete = cascade|set_null|restrict|no_action|deferred_restrict`, `on_soft_delete = cascade`, `backlink = posts`, `no_foreign_key`, `renamed_from`, `flatten`, `prefix`, `in(Projection)`, `generated = "…"`.

**Keys and links**: `Id<T>` is a UUIDv7 that knows its table; `Id::from_natural_key` derives UUIDv5 for natural-key upserts; composite keys via multiple `#[db(key)]`. Single links are `Id<T>`/`Option<Id<T>>` with generated accessors (`post.author(&db)`), reverse queries (`Post::by_author(id)`), and `backlink` methods (`user.posts()`). Many-to-many with **link properties** is a table:

```rust
#[db::link(name = "post_tags", from = Post as tags, to = Tag as posts)]
pub struct PostTag {
	pub post_id: Id<Post>,
	pub tag_id: Id<Tag>,
	pub position: i16, // a property of the link, not of either side
	#[db(created_at)]
	pub added_at: Timestamp,
}
```

**Composition and polymorphism**: `#[db::mixin]` + `#[db(flatten)]` (+ `prefix = "billing_"`) flattens shared columns, indexes, and uniques into every table (built-ins: `Timestamps`, `SoftDelete`, `Versioned`). `#[db::polymorphic(name = "actors", public)]` + `extends = Actor` is **class-table inheritance**: a shared supertype table keyed by kind, policies inherited by every variant, `Id<Actor>` links to any variant, `Actor::query().only([ActorKind::Team])`, `id.downcast::<Team>()`, `match budget.owner(&db).await? { Actor::Person(p) => … }`.

**Views**: `#[db::view(name, sql = "…", materialized, reads(Post, User), public)]`, per-dialect SQL, indexes on materialized views (unique → `refresh_concurrently`), select-only policies — full query surface on top.

**Policies, audit, hooks, personas**

```rust
#[table(name = "articles", public)]
#[db::policy(select, |article, cx| article.published.eq(true)
	| cx.global(|viewer: &Viewer| article.author_id.eq(viewer.0)))]
#[db::policy(insert, update, delete, |article, cx| cx.global(|viewer: &Viewer| article.author_id.eq(viewer.0)),
	message = "Only the author can change an article")]
pub struct Article { … }
```

Gel semantics — default-deny, deny wins, reads silently filter, writes checked and failed with `DbError::Forbidden` — plus two things Gel's policies don't have: the same rules compile to **Postgres RLS** (`policies = "rls"`, `register_setting!` → `current_setting('leptoskit.…')`) for use from any client, and personas (`register_persona!`) view the database as an identity in the studio. `cx.count(query)`, `cx.exists(query)`, `cx.setting_list` are available in rules. `#[db::table(audit)]` records every change (actor, reason, before/after, secrets redacted) to `_leptoskit_audit` in the same transaction. `hooks(before_write, after_insert, after_update, after_delete)` are Gel's triggers, typed and in-transaction.

**Search, vectors, geo**: full-text with weights A–D per column, websearch syntax (`"phrases"`, `-excluded`, `OR`, `prefix*`), `highlight`/`excerpt` with segments, ranking and indexes on both dialects (tsvector+GIN / FTS5+bm25). Vectors: six distance kinds, `hnsw`/`ivfflat`, embedding **freshness fingerprints** with `stale_embeddings()` + `embed_stale(…)`, and first-class **hybrid** RRF search of text + vector together. Geo: `Geography<T>` and `Geo<T, Srid<N>>` with type-level coordinate systems (cross-system comparisons don't compile), spatial filters, `nearest`, GeoPackage interop.

**Live queries**: `#[db::table(live)]` captures changes by triggers or logical replication (`publications.rs` is that replication publication, not software releases), the server exposes `db.changes()` as a versioned feed, and `Tracked` connections record read sets so `use_live` subscriptions in the client re-run exactly the queries a write could have changed.

### 3.2 Queries and writes

The builder is closures over a generated `Columns` struct, so filters are type-checked at compile time — no codegen step, no string language:

```rust
let top = User::query()
	.filter(|user| user.karma.gte(10) & user.handle.contains("a"))
	.order_by(|user| user.karma.desc())
	.limit(20)
	.all(&db)
	.await?;
```

Selection is `#[derive(db::Shape)]` structs — nested, one SQL statement, policies at every level:

```rust
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
```

Writes: `New… { … }.insert(&db)` (returns the row), `.upsert(&db, |user| user.handle)` (conflicts on any unique index, partial ones included), `row.save(&db)` (bumps `updated_at` and `version`; optimistic concurrency fails with `DbError::Stale`), `T::update().filter(…).set(…).run(&db)`, soft `delete` / hard `destroy`, link writes `post.tags().add(&db, tag.id, PostTagProperties { position: 0 })` / `.set(&db, …)`. Transactions: `db.transaction(async |tx| { … })`. Read-only helpers take `&impl db::Read` so writes don't compile.

### 3.3 Migrations (today; §8 has the deep dive)

`leptoskit db migration create <name>` diffs the exported schema against the last committed snapshot and writes `migrations/<timestamp>_<slug>/` with `migration.sql` (editable) and `schema.json`; `registry.txt` holds the authoritative order. The server embeds them with `migrations!`, applies pending ones at startup, one transaction each, recorded with a SHA-256 of the SQL in `_leptoskit_migrations`. Renames resolve from `renamed_from` hints; destructive steps refuse without `--allow-destructive`; `db push` mutates a dev database directly and a later `migration create` adopts the pushes into one migration.

---

## 4. One domain, two spellings

The same blog core — users, posts, tags with an ordered link, a computed name, constraints, a policy, full-text and vector search — first as Gel SDL, then as leptoskit.

**Gel:**

```edgeql
module default {
	global current_user_id: uuid;

	abstract type HasTimestamps {
		created_at: datetime {
			default := datetime_current();
			rewrite insert, update using (datetime_of_statement());
		}
		updated_at: datetime {
			rewrite update using (datetime_of_statement());
		}
	}

	type User extending HasTimestamps {
		required handle: str {
			constraint exclusive;
			constraint min_len_value(4);
			constraint regexp(r'^[a-z0-9_]+$');
		}
		required email: str { constraint exclusive; }
		multi posts := (.<author[is Post]);
		index fts::index on ((fts::with_options(.handle, weight_category := fts::Weight.A)));
		access policy own_rows allow all using (.id = global current_user_id);
	}

	type Post extending HasTimestamps {
		required title: str { constraint max_len_value(200); }
		body: str;
		required author: User {
			on target delete delete source;
		}
		multi tags: Tag { position: int64; }
		score := (count(.<post[is Comment]));
		embedding: ext::pgvector::vector<768>;
		index ext::pgvector::hnsw_cosine on (.embedding);
	}
}
```

**leptoskit:**

```rust
#[db::mixin]
pub struct Timestamps {
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

#[table(name = "users", public, search(language = "english"))]
#[db::policy(all, |user, cx| cx.global(|viewer: &Viewer| user.id.eq(viewer.0)))]
pub struct User {
	pub id: Id<Self>,
	#[db(unique(except = deleted_at), min_len = 4, regex = "^[a-z0-9_]+$", search(weight = A))]
	pub handle: String,
	#[db(unique, private)]
	pub email: String,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

#[table(name = "posts", public)]
pub struct Post {
	pub id: Id<Self>,
	#[db(flatten)]
	pub timestamps: Timestamps,
	#[db(max_len = 200)]
	pub title: String,
	pub body: Option<String>,
	#[db(on_delete = cascade, backlink = posts)]
	pub author_id: Id<User>,
	#[db(vector(distance = cosine, index = hnsw, source(title, body)))]
	pub embedding: Option<Vector<768>>,
}

#[db::link(name = "post_tags", from = Post as tags, to = Tag as posts)]
pub struct PostTag {
	pub position: i16,
}
```

What the comparison shows:

- Gel's `rewrite` and `default := datetime_current()` are leptoskit's `#[db(created_at)]`/`#[db(updated_at)]` — ours are attributes on plain columns, Gel's are rewrite rules; equivalent in effect.
- Gel's computed backlink (`multi posts := (.<author[is Post])`) is `backlink = posts` on the link.
- Gel's `score := count(…)` has no leptoskit equivalent today: `generated = "…"` may only read its own row's columns, not links. See §7.
- Gel declares the policy against a `global`; we bind a typed `Viewer` from the request context, and the same rule also compiles to RLS.
- Gel's HNSW index sits next to the vector; our `vector(…)` additionally declares _freshness_ (`source(title, body)`), so stale embeddings are a query (`stale_embeddings()`) rather than a bookkeeping problem. Gel's `ext::ai` is the counterpart, with the provider call inside the database.

---

## 5. The matrix

### 5.1 Gel has it; we don't

| Gel feature                                                | Our status                                                            | Worth adding?                                                            |
| ---------------------------------------------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| Path traversal in filters (`.author.name = 'Alice'`)       | Filters compare columns only; traversal is accessors or nested shapes | **Yes — top gap.** Type-checkable via the same `Columns` tree (see §7.1) |
| `multi` properties (`multi tags: str`)                     | Arrays are Postgres-only and schema-level; no `Vec<String>` column    | Yes, medium — `Vec<T>` columns over PG arrays / SQLite JSON              |
| Inline computeds in shapes (`post_count := count(.posts)`) | Shapes have `count(link)` but no arbitrary expressions                | Yes — expression fields in shapes (§7.2)                                 |
| Computed pointers that traverse links                      | `generated` is same-row SQL only                                      | Yes, as virtual shape fields first; stored traversed computeds later     |
| Aggregates: `sum/min/max/mean/stddev` in queries           | Only `count`/`exists`; everything else is a view                      | **Yes — top gap** (§7.2)                                                 |
| `group by` (with `ROLLUP`/`CUBE`)                          | Views only                                                            | Later; views cover it, awkwardly                                         |
| Upsert with a custom `else (update …)`                     | `upsert` updates with the new values                                  | Small; low priority                                                      |
| `update … set { tags += … }` on multi links                | `links.add/remove/set` cover it explicitly                            | No — different, fine                                                     |
| `range`/`multirange` types                                 | Missing                                                               | Low; few apps ask                                                        |
| `duration`, local datetime/date/time, `sequence`           | Missing (`Timestamp` is UTC only)                                     | Yes for `Duration` + local types; `sequence` is a nice small add (§7.5)  |
| Regex in filters                                           | Regex only as column validation                                       | Small add, Postgres-first (§7.4)                                         |
| JSON path filters (`json_get`, `@>` …)                     | `Json<T>` stores and returns, never filters                           | Yes, medium (§7.4)                                                       |
| Schema-defined functions                                   | Hooks + views; no callable DSL                                        | No — Rust is the function language here; that's the point                |
| Branch-aware migration rebase (`gel branch rebase`)        | `branch_databases` exist, migration rebase doesn't                    | Interesting later, after §8.4                                            |
| `upgrade-check` against another server version             | N/A (we _are_ the server)                                             | No                                                                       |
| Exclusion constraints                                      | Missing                                                               | Low                                                                      |
| Collation control                                          | Missing                                                               | Low                                                                      |
| Window functions                                           | Missing                                                               | Gel doesn't have them either; no                                         |

### 5.2 We have it; Gel doesn't

| Ours                                                                             | Why Gel can't                                                  |
| -------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| Compile-time typed queries with zero codegen                                     | EdgeQL is a string language; typed access needs client codegen |
| One policy source → ORM filtering **or** Postgres RLS                            | Policies only apply through Gel's own protocols                |
| Soft delete, `with_deleted()`, soft cascades                                     | No soft-delete concept                                         |
| Account merge (`on_account_merge` strategies)                                    | Nothing comparable                                             |
| Optimistic concurrency (`version` → `DbError::Stale`)                            | Nothing comparable                                             |
| Audit trail as a table attribute                                                 | Build it yourself                                              |
| Live queries + tracked read-set invalidation + `use_live` in the UI              | No live queries at all                                         |
| Materialized views with indexes and `refresh_concurrently`                       | Gel has no materialized views                                  |
| Hybrid text+vector RRF search as one query                                       | Assemble manually                                              |
| Geo with type-level SRIDs; GeoPackage interop                                    | `ext::postgis` exists but untyped by comparison                |
| SQLite — same schema, same queries, embedded                                     | Postgres only                                                  |
| Public projections (`public`, `projection`, `secret`, JSON Schema, `validate()`) | Client types are generated per query, not a table surface      |
| Studio with personas                                                             | Nothing comparable                                             |
| Tracked `Statement::reads::<T>()` for hand-written SQL                           | N/A                                                            |

### 5.3 Same idea, different shape (no action)

Gel's triggers ≈ our hooks (ours are typed Rust in the transaction). Gel's globals ≈ `cx.global` + `register_setting!` (ours also reach RLS). Gel's aliases ≈ non-materialized views. Gel's `on target delete` ≈ `on_delete` (ours map to real FK actions). Gel's abstract types ≈ mixins (ours flatten physically; Gel's share an inheritance hierarchy — both legitimate). Gel's interactive rename prompts vs our `renamed_from` hints: **ours is better for CI** — hints are code-reviewed and replayable where Gel's answers live in a terminal transcript.

---

## 6. Queries, side by side

Each pair is the same question. Gel on the left of the page, leptoskit under it.

**Filter, order, paginate**

```edgeql
select User { handle, karma }
filter .karma >= 10 and contains(.handle, 'a')
order by .karma desc
limit 20;
```

```rust
let top = User::query()
	.filter(|u| u.karma.gte(10) & u.handle.contains("a"))
	.order_by(|u| u.karma.desc())
	.limit(20)
	.all(&db)
	.await?;
```

**A nested fetch, one round trip**

```edgeql
select User {
	handle,
	latest := (select .posts order by .created_at desc limit 3) {
		title, tags: { name }
	},
	pinned := (select .posts filter .pinned = true) { title },
}
filter .handle = 'ada';
```

```rust
let card: ProfileCard = User::query()
	.filter(|u| u.handle.eq("ada"))
	.shape()
	.one(&db)
	.await?;
// ProfileCard { handle, post_count: i64,
//   latest: Vec<PostTitle> where PostTitle { title, tags: Vec<TagName> },
//   pinned: Option<PostTitle> }
```

**Traversal in a filter** — the gap, and what it should become

```edgeql
select Post filter .author.handle = 'ada' and exists .tags;
```

```rust
// Today: fetch the user first, and "has a tag" is not a filter at all —
// it's a per-row link count, or a view.
let ada = User::query().filter(|u| u.handle.eq("ada")).one(&db).await?;
let posts = Post::by_author(ada.id).all(&db).await?;
let tagged = post.tags().count(&db).await? > 0;
```

§7.1 proposes the fix, so it reads:

```rust
let posts = Post::query()
	.filter(|p| p.author.handle.eq("ada") & p.tags.any())
	.all(&db)
	.await?;
```

**Aggregate** — today a view, proposed inline (§7.2)

```edgeql
select User { karma } order by .karma desc;
select sum(User.karma);
```

```rust
// Today: declare once…
#[db::view(name = "karma_stats", sql = "SELECT sum(karma) AS total FROM users")]
pub struct KarmaStats { pub total: i64 }

// Proposed: Sum(u.karma) / shape field #[db(sum(karma))]
let total: i64 = User::query().aggregate(|u| Sum(u.karma), &db).await?;
```

**Insert, upsert, update, nested write**

```edgeql
insert User { handle := 'ada', email := 'ada@example.org' };
insert User { handle := 'grace' } unless conflict on .handle;
update User filter .handle = 'ada' set { karma := .karma + 1 };
insert Post { title := 'Hello', author := (select User filter .handle = 'ada') };
```

```rust
let ada = NewUser { handle: "ada".into(), email: "ada@example.org".into(), … }.insert(&db).await?;
let grace = NewUser { handle: "grace".into(), … }.upsert(&db, |u| u.handle).await?;
User::update().filter(|u| u.handle.eq("ada")).set(|u| u.karma, ada.karma + 1).run(&db).await?;
let post = db.transaction(async |tx| {
	NewPost { author_id: ada.id, title: "Hello".into(), … }.insert(tx).await
}).await?;
```

(`set` takes a value, not an expression over the column; increment-by-one reads the row first, `user.karma += 1; user.save(&db)`, as in the guide.)

**Polymorphic**

```edgeql
select Actor { [is Team].name } filter Actor is Team;
```

```rust
let teams = Actor::query().only([ActorKind::Team]).all(&db).await?;
let team: Team = id.downcast::<Team>()?;
```

**Full-text search, with explanation**

```edgeql
with r := fts::search(Article, 'passkeys rust')
select r.object { title } order by r.score desc limit 10;
```

```rust
let hits = Article::query()
	.search("passkeys rust")
	.highlight(|a| a.title)
	.excerpt(|a| a.body, 20)
	.limit(10)
	.shape::<ArticleCard>()
	.hits(&db)
	.await?;
```

**Vector nearest, and hybrid**

```edgeql
select Article { title }
order by ext::pgvector::cosine_distance(.embedding, <v>$q) empty last
limit 10;
```

```rust
let near: Vec<(Article, f32)> = Article::query()
	.nearest(|a| a.embedding, &query_vector)
	.with_distance()
	.limit(10)
	.all(&db)
	.await?;

let blended = Article::query()
	.hybrid(|h| h.text("passkeys").vector(|a| a.embedding, &query_vector))
	.candidates(50)
	.hits(&db)
	.await?; // HybridHit { row, score, text_rank, vector_rank, distance }
```

**Live**

```text
Gel: no equivalent exists.
```

```rust
// Server: an ordinary query function is subscribable unchanged.
#[rpc::query]
async fn messages(channel: String, db: &Db) -> db::Result<Vec<MessageRow>> { … }

// Client: re-runs exactly when a write could change its result.
let feed = use_live::<messages>(move || messages { channel, limit: 100 });
```

The pattern across all of it: **where Gel puts a language, we put typed builders, and mostly win** — except on paths and aggregates, which are the two places EdgeQL's language shape is genuinely ahead of a closure-over-columns builder. Those two are the value adds; everything else in §5.1 is type-and-feature backfill.

---

## 7. What I'd add to the schema

Ordered by leverage. 1–2 are the ones users will feel daily; 3–5 are completeness; the rest is taste.

### 7.1 Path filters and link existentialism (the big one)

Let a filter closure descend through links; compile to a correlated `EXISTS`. The `Columns` tree already exists per table; generate the same tree behind each link accessor:

```rust
Post::query()
	.filter(|p| {
		p.author.handle.eq("ada")           // EXISTS (SELECT 1 FROM users u WHERE u.id = p.author_id AND u.handle = 'ada')
			& p.tags.any(|t| t.name.eq("rust")) // EXISTS (… post_tags JOIN tags …)
			& p.comments.any()                   // EXISTS (…)
			& !p.editor.is_not_null()
	})
	.all(&db)
	.await?;
```

Design notes: `Col` nodes for links become `LinkCol<T>` exposing the target's `Columns` plus `any(filter) -> bool` and `any() -> bool`; the compile-time type story carries over unchanged (the closure stays typed per table); policies still apply inside the subquery because it targets the linked table's real `query()`; `Tracked`/`eval.rs` need a new `Expr::Exists { link, expr }` arm so live invalidation keeps working. This one feature erases most of EdgeQL's remaining ergonomic lead.

### 7.2 Aggregates in queries and shapes

- `Query::aggregate`: `User::query().filter(…).aggregate(|u| Sum(u.karma), &db)` for `Sum/Min/Max/Avg/CountDistinct`, tuple returns for several at once.
- Shape fields: `#[db(sum(karma))]`, `#[db(avg(score))]`, `#[db(min(created_at))]` next to today's `#[db(count(posts))]`, which is already this feature's special case.
- Leave `group by` to views for now; a `#[derive(db::Group)]` shape over a `group by` key set is a later, optional step (Gel's `ROLLUP`/`CUBE` is a report-writer feature, not an app feature).

### 7.3 Expression fields in shapes (Gel's inline computeds)

```rust
#[derive(Shape, Serialize)]
#[db(from = User, public)]
struct UserCard {
	handle: String,
	#[db(expr = "|u| concat(u.handle, '@example.org')")]
	email_alias: String,
	#[db(expr = "|u| u.karma * 2")]
	double_karma: i64,
}
```

Rendered into the select list, so no extra round trip. A typed closure over the same `Columns` keeps it compile-checked; the expression grammar starts with comparisons/arithmetic/concat and grows on demand. Stored traversed computeds (`score := count(.comments)`, Gel-style) can then be "a virtual field on the table" rather than a new storage concept.

### 7.4 Filter backfill: regex, JSON paths, distinct

- `col.matches_regex(r)` — Postgres `~`, refused on SQLite (or degraded to `contains` with a lint) rather than silently diverging.
- JSON path filters on `Json<T>` columns: `col.json_get("theme").eq("dark")`-shaped, mapping to `->>`/`json_extract` per dialect.
- `.distinct()` on `Query` and shape `Vec` fields.

### 7.5 Type backfill: `Duration`, local date/time, `sequence`

- `Duration` (PG `interval`; SQLite ISO-8601 text) — needed the moment anyone schedules anything.
- `LocalDate`/`LocalDateTime`/`LocalTime` (birthday-on-the-calendar semantics that UTC mangles; PG `date`/`timestamp`/`time` are already local-shaped, SQLite stores text).
- `#[db(sequence)] n: i64` for human-facing monotonic numbers (invoice numbers) distinct from the UUIDv7 key — Gel's `sequence`, one attribute.

### 7.6 Not now (recorded so we stop relitigating)

Schema-defined query functions (Rust is the function language), window functions (Gel lacks them too), collation control, exclusion constraints, ranges/multiranges, hash/SP-GiST/BRIN index selection (the defaults we emit are right for app workloads), and `ext::ai`-style in-database embedding calls (our `embed_stale` pipeline keeps the provider call in app code, where keys and retries belong).

---

## 8. Migrations: the deep dive

### 8.1 What we make today

The pipeline, from `leptoskit_db_schema/src/lib.rs`:

```text
previous snapshot ──┐
                    ├── diff ──► Plan ──► render ──► migration.sql
target schema ──────┘                                schema.json
```

- **The baseline is always a committed snapshot, never a live database.** This is a better foundation than Gel's (whose differ needs a server); we can diff in CI with nothing running.
- Each migration is a directory: editable `migration.sql` + the resulting `schema.json`; `registry.txt` is the authoritative order (comments allowed). Applied state is `_leptoskit_migrations(name, checksum, applied_at)`; one transaction per migration; Postgres wraps the run in an advisory lock; SQLite runs rebuild-migrations with foreign keys off and a pre-commit `pragma_foreign_key_check`.
- Every step carries a `Risk`: `Safe`, `MayFail` (rendered as a `-- may fail:` note, e.g. a new unique index over existing rows), `LosesData` (refuses without `--allow-destructive`). Steps are phased "dependencies before dependents, destruction last."
- `db push` + adopt already implements "change the structs, keep coding": dev writes go straight to the database and a later `migration create` squashes them into one migration recorded as applied.
- Verification: 120 insta snapshots of diff/render scenarios, apply tests against real SQLite and (in CI) Postgres asserting migrated-vs-fresh schema equality, and rollback tests.

This is already past Gel in several ways (snapshot-based diff, checksums that make applied history immutable, partial uniques, dual dialect with correct SQLite rebuilds). The questions you asked, answered:

**What are we making?** A snapshot-diff migration system with per-migration SQL, checksums, and risk classification. What's missing for seamlessness is lifecycle, not diffing.

**If you rename a field, what happens?** You write `#[db(renamed_from = "colour_scheme")]` on the field (or table). The diff emits `RenameColumn` — `ALTER TABLE … RENAME COLUMN` on Postgres, a positional column map inside the table rebuild on SQLite — and **strips the hint from the committed snapshot**, so the hint is once, not forever. Rename candidates must be type-compatible; a same-typed disappearance+appearance asks. Without a hint: a TTY asks (`RenameResolver::Ask`), CI and push fail with `AmbiguousRename` (`RequireHints`), `db check` treats it as drop+add (`NoRenames`), and `--allow-destructive` accepts the data loss. Existing indexes/constraints on the column are matched through the rename rather than dropped and recreated. Gel handles the same moment with confidence-scored interactive prompts and no schema annotation; our hint-in-code is reviewable and replayable in CI, which is strictly better for a repo where migrations are artifacts.

**How do we keep migrations minimal?** Today: push-then-adopt collapses a whole dev session into one migration, and hint-stripping keeps snapshots clean. §8.4 makes minimality structural.

### 8.2 What's missing from "seamless"

1. **Migrations stack without bound.** Nothing ever consolidates. Ten changes mid-release are ten directories, and the folder grows forever — the exact problem you named.
2. **No release awareness.** A migration doesn't know which release it belongs to, whether it has shipped, or whether it is still mutable. monochange knows all three things and is never consulted.
3. **No adoption semantics for consolidation.** `_leptoskit_migrations` treats an unknown applied row as an error (`Unknown`) and an edited one as an error (`Changed`). Right for history; it means any squashing feature must add a first-class "these names were folded into that one" record, or fresh and existing databases can't converge.
4. **Data migrations are SQL-only** (`--empty` + hand edits). Fine — Rust bodies would tie history to a compiler version — but consolidation must _know_ which migrations were hand-edited, because those are the ones whose DML must not be discarded for fresh databases.
5. Small frictions: no `fill_expr`-style backfill prompt when a required column appears (today: hard error naming `NotNullWithoutDefault`, remedy is `#[db(default(sql = …))]`); no `CREATE INDEX CONCURRENTLY` escape hatch (indexes build inside the migration transaction).

### 8.3 The Pina and monochange precedents

**Pina** makes published history immutable and pinned to deployments. Its `migrations/manifest.json` holds every version of every contract; `migrations/publications.json` receipts pin the exact schema hashes each deployment made live; after publication nothing below the pin can change (`PublishedPayloadChanged`, `ProgramIdentityChanged`). A draft is freely replaced in place; **publication is the freezing event**. Ambiguous renames fail closed with `--rename value:points` / `--assume-removed value` / `--manual value` flags, and CI replays the answers from config. Versions are per contract, the width freezes at first publication, and there is deliberately no history pruning — because on-chain accounts can't be enumerated.

**monochange** defines when a release happens: changesets carry intent, `monochange prepare` computes versions and writes release records under `.monochange/releases/<id>/release.json`, and the repo's crates release in one lockstep group. Release = the moment the migrations folder should freeze and consolidate. Nothing in the DB layer talks to it today.

The design below is Pina's freeze rule, expressed through monochange's release event, implemented with the snapshot machinery we already have.

### 8.4 Release-aware consolidation (the design)

Three additions: **release markers**, **consolidate**, and **amend-unreleased**. One invariant underneath: _migrations above the last release marker are immutable history; migrations below it are a single mutable draft._

#### Markers: `registry.txt` learns about releases

```text
# The migrations, in the order they apply. `leptoskit db migration create` adds to it.
20261030T070558_init
20261031T130000_release_1_4_0
# release 1.4.0 (monochange:rel_20261031T120000) snapshot 3f9a…c21d
20261105T081000_release_1_5_0
# release 1.5.0 (monochange:rel_20261105T080000) snapshot 8d2e…97af
```

- A marker sits **below the migrations it froze** and names the monochange release id plus the SHA-256 of the `schema.json` the release ends at — the baseline for everything after it (Pina's pin, one line). `init` kept its name because at 1.4.0 it was the only migration — there was nothing to consolidate.
- Everything **above the last marker** is published: `migration create` refuses to insert above it, checksums and ordering are already enforced, and renaming/deleting those directories fails.
- Everything **below** the last marker is unreleased: freely amended, replaced, or consolidated. In the file above there is nothing below `# release 1.5.0`, so the next `migration create` starts a fresh draft there.

#### `leptoskit db migration consolidate`

Run at release preparation (before the monochange release commit, or as a repo-defined `monochange run` step):

1. Take every migration below the last marker.
2. **Re-diff the baseline snapshot (at the marker) → current target schema** and render one consolidated `migration.sql`. This is a fresh diff, not a concatenation: add-then-drop vanishes, rename chains collapse, intermediate table rebuilds never happen, SQLite gets one rebuild instead of four. The resulting `schema.json` equals today's target.
3. **Hand-edit detection**: for each folded migration, re-render its expected SQL from `diff(prev snapshot, its snapshot)` and compare with the `migration.sql` on disk. A mismatch means hand-written statements — a data migration. Choices: `--keep <name>` leaves it out (it stays in the registry, in order, after the consolidated migration), or refuse and name it. Gel's `--squash` silently discards such DML and hands you a `fixups/` file to run by hand on other instances; refusing (or `--keep`) is the honest version, and fresh-database correctness is why.
4. Write `<ts>_release_<version>/` with `migration.sql`, `schema.json`, and **`replaces.json`**: the folded names with their recorded checksums.
5. Rewrite `registry.txt`: the folded lines are replaced by the one consolidated entry, appended below the previous marker; the release's own marker is appended beneath it when the release commits.

#### Adoption: how every database converges

`pending()` grows one rule: an applied migration whose name appears in some migration's `replaces.json` is _merged_, not `Unknown`.

- **A database that already reached this release's schema** (it applied all the folded names): the consolidated migration is **recorded without running** — the existing `Record::Adopted` mechanism from the push flow. Its `schema.json` hash proves equivalence; `db check` can verify the live schema against it.
- **A database that applied only some** of the folded set: refuse, with the names. This shouldn't occur if markers only land at releases (a database that got release 1.4.0 ran everything above that marker); the error exists so partial states are surfaced, not guessed at.
- **A fresh database**: runs the consolidated migration directly. Intermediate schemas only ever existed below a marker, so no database needs them.

Result: the folder holds **one migration per release that changed the schema**, plus any `--keep` data migrations — tens of entries after years, not thousands. Full fidelity history remains in git for anyone who needs to know what happened mid-release.

#### Amend-unreleased: the Convex feel, kept honest

With `[db] migrations = "amend"` (opt-in, then likely default), `migration create` **amends the single migration below the last marker** instead of stacking: re-diff baseline → target, rewrite it, and adopt in the local dev database (the push machinery already adopts). During development you edit structs and the one draft migration keeps up; at release, `consolidate` is usually a no-op rename (the draft already _is_ the release migration — it just needs its marker). `--new` forces today's stacking behavior when a change deserves its own reviewed migration (a destructive step, a data migration).

This is deliberately more than Gel offers: `gel watch --migrate` reconciles dev databases, but every `migration create` still stacks, and squashing is a separate destructive act you must remember. Here, "the migrations folder is one entry per release" is the _default shape_, with git holding the churn.

#### What stays out

- No database-side migration objects or `extract`-from-history: our snapshots make the repo self-sufficient.
- No cross-database fixups: with `replaces.json` adoption there is nothing to fix up.
- No consolidation _above_ markers, ever — that's Pina's published-immutability rule, and the answer to "when the last release was" is always readable from the registry itself.

### 8.5 Implementation sketch

| Piece               | Where                                                                    | Notes                                                                                                                                                           |
| ------------------- | ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Marker parse/format | `db_schema` (`registry` handling in `cli/src/db.rs`)                     | Comment-line grammar, snapshot hash line; `History::read` exposes `baseline_at_last_marker()`                                                                   |
| Freeze rules        | `cli/src/db.rs` create/edit paths + `db_schema::diff`                    | Refuse inserts above marker; registry lint in `migrations!`                                                                                                     |
| Consolidate         | new `db_schema` fn `consolidate(baseline, target, folded) -> Plan` + CLI | Mostly re-uses `diff::plan`; hand-edit detection via per-migration re-render                                                                                    |
| `replaces.json`     | `db` (`migrate.rs`)                                                      | Extend `pending()` with merged/adopted states; reuse `Record::Adopted`                                                                                          |
| Amend mode          | `cli/src/db.rs` + config `[db] migrations = "amend"`                     | Adopt-in-dev reuses `push::prepare_squash`                                                                                                                      |
| monochange wiring   | `monochange.toml` `[cli.release]`-style step or repo workflow            | `db migration consolidate` then `db release record <version>` writing the marker; release records already cite commits                                          |
| Tests               | `db_schema/tests/diff__*` + apply tests + `db/tests/migrations.rs`       | New scenarios: consolidate-collapses-chains, hand-edit-refusal, `--keep` ordering, partial-applied refusal, fresh-vs-consolidated equality, marker immutability |

Guardrails to keep: consolidated output must satisfy the same insta snapshot + apply-on-real-databases equality tests as any migration; `consolidate` is refused when any folded migration is applied to the _local_ database's marker state inconsistently; and `db check` grows a drift mode comparing the live schema against the last applied snapshot, so "seamless" includes noticing when someone hand-touched production.

---

## 9. Plan

Phased, smallest-first; phases 1–3 are the migration lifecycle, phase 4 is the schema surface. Each is independently shippable.

| Phase                 | Deliverable                                                                                                                                      | Proves                                                                                                                           |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------- |
| 1. Markers            | `# release` markers in `registry.txt`, freeze rules above the last marker, `db release record`                                                   | Published history is immutable; CI catches an attempt to edit it                                                                 |
| 2. Consolidate        | `db migration consolidate` with `replaces.json`, hand-edit detection, `--keep`                                                                   | A folder of N mid-release migrations becomes 1; fresh and already-migrated databases converge (apply tests on SQLite + Postgres) |
| 3. Amend              | `[db] migrations = "amend"` + adopt-in-dev                                                                                                       | A dev session of schema edits leaves one draft migration; release turns it into the release migration                            |
| 4. Paths & aggregates | `LinkCol` traversal + `any()` in filters; `aggregate` + `#[db(sum/min/max/avg(…))]` shape fields; `matches_regex`, JSON path filters, `distinct` | The two EdgeQL capabilities we lack; snapshot tests per dialect, `Tracked`/`eval` arms for invalidation                          |
| 5. Type backfill      | `Duration`, `LocalDate/LocalDateTime/LocalTime`, `#[db(sequence)]`                                                                               | Column types round-trip on both dialects; JSON Schema updates                                                                    |

Phases 4–5 are separable from 1–3 and from each other; if only one thing ships from this document, ship phase 2 — the stacking problem is the one that compounds.

---

_Sources: Gel feature claims from docs.geldata.com (datamodel, EdgeQL, migrations references; changelogs 6.x/7.x) and the gel-joins-vercel announcement, verified November 2026. leptoskit claims from `crates/leptoskit_db_schema`, `crates/leptoskit_db`, `crates/leptoskit_db_macros`, `crates/leptoskit_cli/src/db.rs`, and `docs/src/guide/22-database.md` at the commit above. Pina semantics from the Pina skill's migrations reference; monochange semantics from the monochange skill._
