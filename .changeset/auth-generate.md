---
"release": minor
---

# Generated sign-in screens and an `--auth` scaffold

Sign-in screens are the same few pieces in every app, so leptoskit now writes them for you, built on the headless client so they work in the browser and in native apps alike.

- **`leptoskit generate auth`** writes an email-and-code sign-in screen and a "where you're signed in" screen into your app's `src/auth/`, using leptoskit's own components and Tailwind classes. They show loading and error states, and after too many attempts they disable the button until the server's wait is over. It also adds `[auth]` to `leptoskit.toml`, the `auth`, `leptos`, and `query` features, `/sign-in` and `/devices` routes when the app uses the file router, and a `sign_in` module for a server that has a database, then lists what's left to wire by hand. Running it again changes nothing, and a screen you've edited is kept unless you pass `--force`.
- **`leptoskit new --auth`** starts a project with sign-in wired up: the `[auth]` table, a `User` table keyed by the account and filled in by `OnSignUp`, the server's routes and layer, the development mailbox and dev sign-in in debug builds, the client configured at startup, and the sign-in screens in front of the app. It adds a SQLite database unless `--db` picks one.
