# Merging accounts

Someone signs up with GitHub, writes a dozen posts, and months later taps "Continue with Google" with the same address. Now they have two accounts. leptoskit never joins them on its own, because trusting a matching email is how accounts get taken over. Instead the person connects the second sign-in method to their main account, proves they control the other account, and chooses what happens to its data.

## Where an offer comes from

Connecting a provider (`link_provider()`) or [linking a wallet](wallets.md) (`link_wallet()`) that already belongs to another account here makes a merge offer, for ten minutes, on the signed-in device. A wallet only makes one when it may sign in to the other account, since only then is signing with it as good as signing in there.

`use_merge_offer()` describes the other account:

```rust
use leptoskit::auth::client::use_merge_offer;

let offer = use_merge_offer();   // Some(MergeOffer) while there's an offer
```

| Field           | What it says                                                                                 |
| --------------- | -------------------------------------------------------------------------------------------- |
| `created_at`    | When the other account was made                                                              |
| `emails`        | Its verified addresses, which move here                                                      |
| `identities`    | Its connected providers, which move here                                                     |
| `passkeys`      | How many passkeys move here                                                                  |
| `wallets`       | Its linked wallets' addresses, which move here                                               |
| `data`          | Its data, table by table: `MergeData { label, count, fate }`, such as "12 posts"             |
| `second_factor` | Its second factors; one of them is needed to merge, so a merge can't get around them         |
| `modes`         | What the merge may do with its data: `BringData` (when your tables allow it) and `MoveLogin` |

The generated security screen shows it as a choice:

```text
That login already has its own account
It was created on 3 March 2026. Merging moves its sign-in methods here (old@example.com, GitHub, 1 passkey).

● Bring its data here, then delete it
  Moves 12 posts. Moves 2 follows, except what you already have. Keeps the newer of the two prefs.
○ Move only the sign-in methods, and delete its data

[Cancel]  [Merge accounts]
```

## Merge

```rust
use leptoskit::auth::MergeMode;
use leptoskit::auth::client::MergeInput;
use leptoskit::auth::client::merge_accounts;

let merge = use_mutation(merge_accounts());
merge.mutate(MergeInput {
	mode: Some(MergeMode::BringData),
	confirm: true,
	code: Some(code_from_its_authenticator),   // when the other account has two-step verification
	..MergeInput::default()
});
```

Both modes, in one transaction:

1. Move the other account's verified email addresses, connected providers, passkeys, and wallets here. Its authenticator and recovery codes are discarded; this account's stay.
2. With `BringData`, move its rows in your tables here, each table as its `on_account_merge` strategy says (see [when two accounts merge](../22-database.md#when-two-accounts-merge)), and run your `OnMerge` for the tables you move yourself. With `MoveLogin`, its rows are deleted with it through `on_delete = cascade`.
3. Sign it out everywhere, delete it, and leave a tombstone, so `auth.resolve_merged(old_id)` returns this account for links you stored.
4. Record it in this account's history, which keeps the other account's history too, and email this account's primary address.

If anything fails, such as a row that breaks one of your unique constraints, nothing changes, and the merge answers `merge_conflict`. It can't be undone once it succeeds, which is why it needs `confirm: true`.

## Your own merge code

Tables marked `custom` are yours to move, and `merge_profile` copies what's worth keeping from the other account's `User` before it's deleted. Register your `OnMerge` with `AuthBuilder::on_merge::<User>()`; without it, a `custom` table means only the login can move.

```rust
impl auth::OnMerge for User {
	/// Optional: the preview's own words, instead of counts by table.
	async fn summary(db: &Db, absorbed: Id<auth::Account>) -> auth::Result<Option<Vec<auth::MergeData>>> {
		let posts = Post::query().filter(|post| post.author_id.eq(absorbed)).count(db).await?;
		Ok(Some(vec![auth::MergeData { label: "posts".into(), count: posts, fate: auth::MergeFate::Moves }]))
	}

	async fn merge_profile(tx: &Tx, survivor: Id<auth::Account>, absorbed: Id<auth::Account>) -> auth::Result<()> {
		// Keep the old bio if the main account has none.
		Ok(())
	}

	async fn merge_custom(tx: &Tx, table: &TableName, survivor: Id<auth::Account>, absorbed: Id<auth::Account>) -> auth::Result<()> {
		// `balances`: add the two together.
		Ok(())
	}
}
```

They run with full access to the database, in the merge's transaction: an error from any of them undoes the whole merge.

## On every platform

Merging works the same everywhere: the offer, the preview, and `merge_accounts()` are server calls. Proving the other account with its passkey (`MergeInput { passkey: true, .. }`) needs a browser for now, like other passkey use; a code from its authenticator app works everywhere. Native apps can't connect a provider or link a wallet yet, so offers come from the browser.

## How it's kept safe

- A merge needs a sign-in or step-up within the last 10 minutes on this account, and proof of the other account as strong as signing in to it: its provider sign-in or its wallet's signature, and its second factor when it has one, which counts toward its lockout.
- The offer is bound to the session that made it, lasts ten minutes, and works once.
- Bringing data along only uses the strategies your tables declared, and only once every table that holds an account's data has one, so a merge never fails halfway for the person trying it.
