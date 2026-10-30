---
"release": minor
---

# Polymorphic tables and the database studio

A link can now point at one of several tables. Declare them as an enum with `#[db::polymorphic(name = "actors")]`, one variant per table, and mark each table `extends = Actor`; an `Id<Actor>` column is then an ordinary foreign key, and `budget.owner(&db)` returns whichever kind it points at, for an exhaustive `match`. The enum gets a table of its own, holding each row's key and kind, which inserts and removals of any variant write in the same transaction. `Actor::query()` reads every kind, or `only` some, through each table's own policies and soft deletes, with paging across them all; `Actor::find`, `id.downcast::<Team>()`, and `user.id.upcast()` move between the enum and its tables. Policies on the enum apply to every variant, in the database layer and as row-level security; `public` adds `PublicActor`, tagged by kind, and `live` publishes rows of any kind as they come and go.

`leptoskit db studio` serves a browser UI for the development database, and `leptoskit dev --studio` serves it beside the app. It shows each table's rows, with filters, sorting, and paging; the schema; the migrations and which the database has; the SQL each policy adds; the audit log; and live changes. Every read goes through the app's policies, as nobody, as `system` with a reason you type, or as a persona such as `leptoskit_auth`'s signed-in `account`; register your own with `register_persona!`. Changes need edit mode and a confirmation, and run through each table's own `save` and `delete`. It listens on this machine only, behind a token made for each run, and release builds never contain it.
