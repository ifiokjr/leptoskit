---
"release": minor
---

# Links, shapes, projections, and generated columns

`#[db::link(name = "post_tags", from = Post as tags, to = Tag as posts)]` declares a table that links two others many to many, on a struct of the link's own columns. The macro adds the two ends as the table's key, deleted with either end, and generates `post.tags()` and `tag.posts()`: `add(&db, tag.id, PostTagProperties { position: 0 })`, `remove`, `all` (each tag with its link), `ids`, `count`, and `contains`, with `filter`, `filter_link`, `order_by`, `limit`, and `offset`. A link is only there when the request may see both its ends, and link tables take policies like any table. `exclusive` lets each row be linked from one row only, and a table can link to itself with `from_column` and `to_column`.

`#[derive(db::Shape)]` fetches a row and the rows linked to it in one statement: `#[db(count(posts))]` counts linked rows, and `#[db(via = posts, order_by = |post| post.created_at.desc(), limit = 3)]` nests them as another shape, through link columns, backlinks, and `#[db::link]` tables, on Postgres and SQLite alike. `User::query().filter(…).shape::<ProfileCard>().one(&db)` runs it. Every nested table's policies and soft deletes apply, and a `public` shape fails to compile if it reads a private column or nests a shape that isn't public.

`projection(SelfUser)` on a table, with `#[db(private, in(SelfUser))]` on a column, generates a second client type that shows that private column, such as the signed-in user's own email.

`#[db(generated = "lower(handle)", unique)]` declares a column the database computes from the row's other columns and stores, so it can be indexed and unique; `generated(postgres = "…", sqlite = "…")` gives each database its own expression. Inserts, upserts, and saves never write it and return the computed value, and setting one in a bulk update fails. Migrations add and drop them on both databases.
