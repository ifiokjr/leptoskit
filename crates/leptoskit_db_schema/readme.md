<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_db_schema

> The schema model, snapshot format, diff, and SQL renderers behind leptoskit's generated database migrations.

`leptoskit db` and the `leptoskit_db` runtime use this crate; apps never depend on it. Everything in it is pure: no database connection and no files.

- `Schema` describes what a database should contain: tables with typed columns, primary keys, indexes (unique ones included, optionally partial), foreign keys, checks, comments, and namespaces. It reads and writes the `schema.json` snapshot committed beside each migration, and `validate` reports every inconsistency with its table and column.
- `diff(previous, target, renames)` describes, step by step, how to turn one schema into the other. Renames come from `renamed_from` hints, or from a `RenameResolver` when something appeared while something similar disappeared: `RequireHints` fails so that CI never guesses, and `NoRenames` treats the change as new. Indexes and constraints that only changed name, because their table was renamed, are renamed rather than rebuilt. Every step reports its `Risk` against existing rows, so that data-destroying migrations need explicit consent.
- `render(plan, previous, target)` writes the plan as SQL for its dialect. Postgres alters in place, adds foreign keys once every table exists, and drops tables in one statement. SQLite uses `STRICT` tables with checks that hold each column to its type, and rebuilds a table whenever `ALTER TABLE` can't make the change, using SQLite's documented procedure with foreign keys off and checked before commit.

The baseline for a new migration is always the previous migration's snapshot, never a live database, so generating one gives the same SQL on every machine. The test suite applies every scenario to real SQLite and Postgres databases and checks that a migrated schema matches one created from scratch, with every row kept.
