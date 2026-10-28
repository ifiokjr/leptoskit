<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_db_macros

> The `#[db::table]` attribute and `#[derive(db::Enum)]` behind leptoskit's database layer.

Apps use these through `leptoskit::db` (or `leptoskit_db`); see that crate for what they generate. Every mistake the macros can catch is a compile error that says what to write instead: a table without an explicit name, a derive that would leak secrets, a missing key, an invalid regular expression.
