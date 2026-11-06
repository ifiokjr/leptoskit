---
"release": minor
---

# Filters that follow links, totals, and distinct reads

`Query::aggregate` totals a column over the matching rows — `Sum`, `Avg`, `Min`, or `Max` of a column in one typed statement, with the table's policies applied and `None` when no row matches, as SQL returns. A link column's `has` follows it: `post.author_id.has(|user| user.handle.eq("ada"))` becomes a correlated `EXISTS`, in `filter` and in `!` for its negation, and an optional link that is `NULL` matches nothing. Text columns match regular expressions with `matches_regex` — Postgres's `~`; queries on SQLite refuse it rather than diverge. `distinct()` reads each row once, which matters for views and naturally-keyed tables, and `count()` counts after it.
