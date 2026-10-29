---
"release": minor
---

# Cascade soft deletes, and check links at commit

`#[db(on_soft_delete = cascade)]` on a link soft-deletes its rows when the row it points at is soft-deleted, then follows their own cascading links, in one transaction that joins yours if you're already in one. Single and bulk deletes both cascade, and rows already deleted keep their original time. `#[db(on_delete = deferred_restrict)]` checks a link when the transaction commits, so related rows can be inserted in any order. A soft delete now also moves a versioned row's version.
