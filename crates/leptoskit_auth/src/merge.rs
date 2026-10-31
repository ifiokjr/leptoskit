//! Merging in "bring its data" mode: the absorbed account's rows in the
//! app's own tables move to the survivor, each table by its
//! `on_account_merge` strategy, in the same transaction as the sign-in
//! methods. The schema says which tables hold an account's data, so a
//! typical app writes no merge code; [`OnMerge`] covers the rest.

use std::future::Future;
use std::pin::Pin;

use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Table;
use leptoskit_db::Tx;
use leptoskit_db::merge::AccountData;
use leptoskit_db::merge::MergeStrategy;
use leptoskit_db::merge::TableName;

use crate::Account;
use crate::Auth;
use crate::AuthError;
use crate::MergeData;
use crate::MergeFate;
use crate::Result;

/// What the app does when two accounts merge and the absorbed account's
/// data comes along.
///
/// ```ignore
/// impl auth::OnMerge for User {
/// 	async fn summary(db: &Db, absorbed: Id<auth::Account>) -> auth::Result<Option<Vec<auth::MergeData>>> {
/// 		let posts = Post::query().filter(|post| post.author_id.eq(absorbed.into())).count(db).await?;
/// 		Ok(Some(vec![auth::MergeData { label: "posts".into(), count: posts, fate: auth::MergeFate::Moves }]))
/// 	}
///
/// 	async fn merge_custom(tx: &Tx, table: &TableName, survivor: Id<auth::Account>, absorbed: Id<auth::Account>) -> auth::Result<()> {
/// 		// Add the balances together, for `wallet_balances`.
/// 		Ok(())
/// 	}
/// }
/// ```
///
/// Every method runs as system, bypassing policies, and the last two in
/// the merge's transaction: an error undoes the whole merge.
pub trait OnMerge {
	/// What the merge preview lists, such as "12 posts". `None`, the
	/// default, counts the rows of each table that would move.
	fn summary(
		db: &Db,
		absorbed: Id<Account>,
	) -> impl Future<Output = Result<Option<Vec<MergeData>>>> + Send {
		let _ = (db, absorbed);
		async { Ok(None) }
	}

	/// Copies what's worth keeping from the absorbed account's rows in the
	/// tables keyed by the account, such as its `User`, onto the survivor's.
	/// They're deleted with the account afterwards. Does nothing by default.
	fn merge_profile(
		tx: &Tx,
		survivor: Id<Account>,
		absorbed: Id<Account>,
	) -> impl Future<Output = Result<()>> + Send {
		let _ = (tx, survivor, absorbed);
		async { Ok(()) }
	}

	/// Moves the rows of `table`, which is marked `on_account_merge =
	/// custom`.
	fn merge_custom(
		tx: &Tx,
		table: &TableName,
		survivor: Id<Account>,
		absorbed: Id<Account>,
	) -> impl Future<Output = Result<()>> + Send;
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// [`OnMerge::summary`].
type SummaryHook = for<'a> fn(&'a Db, Id<Account>) -> BoxFuture<'a, Result<Option<Vec<MergeData>>>>;

/// [`OnMerge::merge_profile`].
type ProfileHook = for<'a> fn(&'a Tx, Id<Account>, Id<Account>) -> BoxFuture<'a, Result<()>>;

/// [`OnMerge::merge_custom`].
type CustomHook =
	for<'a> fn(&'a Tx, &'a TableName, Id<Account>, Id<Account>) -> BoxFuture<'a, Result<()>>;

/// An app's [`OnMerge`], as functions.
#[derive(Clone, Copy)]
pub(crate) struct MergeHooks {
	summary: SummaryHook,
	profile: ProfileHook,
	custom: CustomHook,
}

impl MergeHooks {
	pub(crate) fn of<U: OnMerge + 'static>() -> Self {
		Self {
			summary: |db, absorbed| Box::pin(U::summary(db, absorbed)),
			profile: |tx, survivor, absorbed| Box::pin(U::merge_profile(tx, survivor, absorbed)),
			custom: |tx, table, survivor, absorbed| {
				Box::pin(U::merge_custom(tx, table, survivor, absorbed))
			},
		}
	}
}

impl Auth {
	/// What holds an account's data in the app's schema.
	pub(crate) fn account_data(&self) -> Result<AccountData> {
		let schema = leptoskit_db::schema(self.inner.db.dialect())
			.map_err(|error| AuthError::Config(format!("the schema doesn't load: {error}")))?;
		Ok(leptoskit_db::merge::account_data(
			&schema,
			&Account::table_name(),
		))
	}

	/// Whether a merge may bring the absorbed account's data along: every
	/// table says how its rows move, and the app moves the `custom` ones.
	pub(crate) fn can_bring_data(&self, data: &AccountData) -> bool {
		data.is_covered() && (!data.has_custom() || self.inner.on_merge.is_some())
	}

	/// The absorbed account's data, as the preview lists it.
	pub(crate) async fn merge_data(
		&self,
		data: &AccountData,
		absorbed: Id<Account>,
	) -> Result<Vec<MergeData>> {
		let db = &self.inner.db;

		if let Some(hooks) = &self.inner.on_merge
			&& let Some(summary) = (hooks.summary)(db, absorbed).await?
		{
			return Ok(summary);
		}

		let mut counts: Vec<MergeData> = Vec::new();

		for reference in &data.references {
			let count =
				leptoskit_db::merge::count_references(db, reference, absorbed.uuid()).await?;

			if count == 0 {
				continue;
			}

			let fate = fate(reference.strategy);
			let label = reference.table.name.clone();

			// A table that points at the account twice, such as follows,
			// counts once per column.
			match counts
				.iter_mut()
				.find(|existing| existing.label == label && existing.fate == fate)
			{
				Some(existing) => existing.count += count,
				None => counts.push(MergeData { label, count, fate }),
			}
		}

		Ok(counts)
	}

	/// Moves the absorbed account's data to the survivor, in the merge's
	/// transaction, before the absorbed account is deleted.
	pub(crate) async fn bring_data(
		&self,
		tx: &Tx,
		data: &AccountData,
		survivor: Id<Account>,
		absorbed: Id<Account>,
	) -> Result<()> {
		for reference in &data.references {
			leptoskit_db::merge::move_references(tx, reference, absorbed.uuid(), survivor.uuid())
				.await
				.map_err(|error| {
					AuthError::MergeConflict(format!(
						"{}.{}: {error}",
						reference.table, reference.column
					))
				})?;
		}

		let Some(hooks) = self.inner.on_merge else {
			return Ok(());
		};
		let mut custom: Vec<&TableName> = data
			.references
			.iter()
			.filter(|reference| reference.strategy == MergeStrategy::Custom)
			.map(|reference| &reference.table)
			.chain(
				data.keyed
					.iter()
					.filter(|keyed| keyed.strategy == MergeStrategy::Custom)
					.map(|keyed| &keyed.table),
			)
			.collect();
		custom.sort();
		custom.dedup();

		for table in custom {
			(hooks.custom)(tx, table, survivor, absorbed).await?;
		}

		(hooks.profile)(tx, survivor, absorbed).await
	}
}

/// What a strategy does to the rows, for people.
fn fate(strategy: MergeStrategy) -> MergeFate {
	match strategy {
		MergeStrategy::Reassign => MergeFate::Moves,
		MergeStrategy::ReassignSkipConflicts => MergeFate::MovesUnlessDuplicate,
		MergeStrategy::KeepNewest => MergeFate::NewestStays,
		MergeStrategy::KeepSurvivor => MergeFate::Dropped,
		MergeStrategy::Custom => MergeFate::AppDecides,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn each_strategy_has_a_fate_people_can_read() {
		assert_eq!(fate(MergeStrategy::Reassign), MergeFate::Moves);
		assert_eq!(
			fate(MergeStrategy::ReassignSkipConflicts),
			MergeFate::MovesUnlessDuplicate
		);
		assert_eq!(fate(MergeStrategy::KeepNewest), MergeFate::NewestStays);
		assert_eq!(fate(MergeStrategy::KeepSurvivor), MergeFate::Dropped);
		assert_eq!(fate(MergeStrategy::Custom), MergeFate::AppDecides);
	}
}
