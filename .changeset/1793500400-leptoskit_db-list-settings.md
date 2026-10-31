---
"release": minor
---

# Policies can look a column up in a list setting

A request's rules can now ask whether a column is one of several values the request holds, such as the teams someone belongs to, and the rule still runs as Postgres row-level security:

```rust
#[db::policy(select, |project, cx| project.team_id.in_list(cx.setting_list::<Teams>()))]
```

Implement `ListSetting` for the type and register it with `register_list_setting!`. Under row-level security the list travels as an array, so raw SQL is filtered the same way. A setting whose value is empty text now counts as missing even without row-level security, because Postgres can't tell the two apart.
