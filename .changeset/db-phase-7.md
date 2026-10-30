---
"release": minor
---

# Full-text search and row-level security

Mark text columns with `#[db(search)]`, or `#[db(search(weight = A))]` to make a match there count more, and `Article::query().search("passkeys rust")` returns the matching rows best first. The query reads like a search box (`"phrases"`, `-excluded`, `OR`, `prefix*`), and nothing typed into it is ever SQL syntax. `hits()` returns each row's rank and, with `highlight(…)` and `excerpt(…, words)`, its matching words marked as plain-text segments rather than HTML. The table's `select` policy, soft deletes, filters, and paging apply as to any query. Postgres keeps a generated `tsvector` column with a GIN index; SQLite keeps an FTS5 table and a key map that triggers update on every write, raw SQL included. `search(language = "english")` stems words; the default, `simple`, matches them as written.

On Postgres, `[db] policies = "rls"` also enforces access policies as row-level security, so raw SQL is filtered too. Rules read the request through settings, `cx.setting::<Viewer>()`, from types that implement `Setting` and are registered with `register_setting!`; migrations create a policy per command, and `Db::with_row_security()` tells Postgres who each transaction acts for. Results are the same with it on and off. `leptoskit_auth`'s tables read the signed-in account as a setting.
