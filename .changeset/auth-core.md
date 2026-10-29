---
"release": minor
---

# Sign-in: the auth pack, sessions, and email codes

`leptoskit::auth` (the `auth` feature, with `db-postgres` or `db-sqlite`) adds sign-in in the app's own database.

- **Tables:** accounts, verified emails, sessions, single-use challenges, rate limits and an audit trail ship as the `auth` schema pack. Policies show each person only their own rows.
- **Email codes:** one flow signs in to an address's account or creates it, and gives the same answer for every address. Codes are keyed hashes that expire after 10 minutes, work once, and allow five guesses.
- **Sessions:** opaque 256-bit tokens, of which only the hash is stored. They have idle and absolute lifetimes, and support sign-out, revoking one or all other sessions, rotation, and disabling an account.
- **Rate limits:** sending and checking codes is limited per address and per IP address.
- **Policies:** `cx.signed_in(|me| post.author_id.eq(me.user_id()))` lets access policies read the signed-in person.
- **Sign-up:** `OnSignUp` creates the app's user row in the same transaction as the account.
