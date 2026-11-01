//! Which tables hold an account's data, and how each moves when two
//! accounts merge: the analysis behind sign-in's "bring its data" merges and
//! `leptoskit db check`'s guard.
//!
//! An app's data reaches an account two ways:
//!
//! - **Tables keyed by the account**, whose primary key is also a foreign key
//!   to it, such as a `users` table that shares its key with `auth.accounts`,
//!   or a `profiles` table keyed by `users`. Each account has at most one row.
//!   The survivor's row stays, the absorbed account's goes with it, and the app
//!   copies anything worth keeping in a `custom` strategy.
//! - **References**: every other single-column foreign key to the accounts
//!   table or a table keyed by it, such as `posts.author_id`. These rows move
//!   by their table's [`MergeStrategy`], which defaults to `reassign`.
//!
//! A reference inside a unique constraint can't be moved blindly: two rows
//! could collide, and the merge would fail for the person trying it. So such
//! a table must say what to do, and until every one does, the analysis
//! reports a problem.

use std::fmt;

use crate::MergeStrategy;
use crate::Schema;
use crate::Table;
use crate::TableName;

/// One column that points at an account, and how its rows move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountReference {
	/// The table the column is in.
	pub table: TableName,
	/// The referencing column.
	pub column: String,
	/// The table it points at: the accounts table, or one keyed by it.
	pub references: TableName,
	/// How the rows move: the table's own strategy, or `reassign`.
	pub strategy: MergeStrategy,
	/// Whether the table chose the strategy.
	pub explicit: bool,
	/// The unique constraints the column is part of, primary key included,
	/// as each one's other columns: `[[]]` when the column alone is unique.
	pub unique_with: Vec<Vec<String>>,
	/// Whether the table has an `updated_at` column, which `keep_newest`
	/// compares.
	pub has_updated_at: bool,
}

impl AccountReference {
	/// Whether the column alone is unique: one row per account.
	pub fn one_per_account(&self) -> bool {
		self.unique_with.iter().any(Vec::is_empty)
	}
}

/// A table whose primary key is an account's, so each account has at most
/// one row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyedTable {
	/// The table.
	pub table: TableName,
	/// Its key column.
	pub column: String,
	/// The table its key points at.
	pub references: TableName,
	/// `keep_survivor`, the default, or `custom`.
	pub strategy: MergeStrategy,
}

/// Everything that holds an account's data, outside the accounts table's
/// own namespace (sign-in's pack moves its own rows).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountData {
	/// The tables keyed by the account, parents before the tables keyed by
	/// them.
	pub keyed: Vec<KeyedTable>,
	/// Every other column that points at an account.
	pub references: Vec<AccountReference>,
	/// What stops a merge from bringing the data along, if anything.
	pub problems: Vec<MergeProblem>,
}

impl AccountData {
	/// Whether every table says how its rows move, so a merge can bring the
	/// absorbed account's data along.
	pub fn is_covered(&self) -> bool {
		self.problems.is_empty()
	}

	/// Whether any table moves its rows itself.
	pub fn has_custom(&self) -> bool {
		self.keyed
			.iter()
			.any(|table| table.strategy == MergeStrategy::Custom)
			|| self
				.references
				.iter()
				.any(|reference| reference.strategy == MergeStrategy::Custom)
	}
}

/// Why a table can't take part in a merge as declared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeProblem {
	/// The column is part of a unique constraint, and the table has no
	/// strategy: moving its rows could collide.
	NeedsStrategy {
		/// The table.
		table: TableName,
		/// The column.
		column: String,
	},
	/// `keep_newest` needs one row per account and an `updated_at` column.
	KeepNewestNeedsOneRow {
		/// The table.
		table: TableName,
	},
	/// A table keyed by the account keeps the survivor's row, or moves its
	/// rows with `custom`; its key can't be reassigned.
	KeyedTable {
		/// The table.
		table: TableName,
		/// The strategy it asked for.
		strategy: MergeStrategy,
	},
}

impl fmt::Display for MergeProblem {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::NeedsStrategy { table, column } => {
				write!(
					f,
					"{table}.{column} is in a unique constraint, so merging accounts could \
					 collide: add `on_account_merge = reassign_skip_conflicts`, `keep_survivor`, \
					 `keep_newest`, or `custom` to #[db::table]"
				)
			}
			Self::KeepNewestNeedsOneRow { table } => {
				write!(
					f,
					"{table} uses `keep_newest`, which needs one row per account (the reference \
					 alone unique) and an `updated_at` column"
				)
			}
			Self::KeyedTable { table, strategy } => {
				write!(
					f,
					"{table} is keyed by the account, so it keeps the survivor's row: use \
					 `keep_survivor` or `custom`, not `{}`",
					strategy.as_str()
				)
			}
		}
	}
}

/// What holds an account's data in `schema`, where `accounts` is the
/// accounts table.
pub fn account_data(schema: &Schema, accounts: &TableName) -> AccountData {
	let outside = |table: &Table| table.name.namespace != accounts.namespace;
	let mut data = AccountData::default();
	let mut targets = vec![accounts.clone()];

	// Tables keyed by an account, or by a table keyed by one, until no more
	// turn up.
	loop {
		let found = schema
			.tables
			.iter()
			.filter(|table| outside(table))
			.find_map(|table| {
				if data.keyed.iter().any(|keyed| keyed.table == table.name) {
					return None;
				}

				let [key] = table.primary_key.as_slice() else {
					return None;
				};
				table
					.foreign_keys
					.iter()
					.find(|foreign_key| {
						foreign_key.columns == [key.clone()]
							&& targets.contains(&foreign_key.references)
					})
					.map(|foreign_key| {
						KeyedTable {
							table: table.name.clone(),
							column: key.clone(),
							references: foreign_key.references.clone(),
							strategy: table
								.on_account_merge
								.unwrap_or(MergeStrategy::KeepSurvivor),
						}
					})
			});
		let Some(keyed) = found else {
			break;
		};
		targets.push(keyed.table.clone());
		data.keyed.push(keyed);
	}

	for keyed in &data.keyed {
		if !matches!(
			keyed.strategy,
			MergeStrategy::KeepSurvivor | MergeStrategy::Custom
		) {
			data.problems.push(MergeProblem::KeyedTable {
				table: keyed.table.clone(),
				strategy: keyed.strategy,
			});
		}
	}

	for table in schema.tables.iter().filter(|table| outside(table)) {
		let key = data
			.keyed
			.iter()
			.find(|keyed| keyed.table == table.name)
			.map(|keyed| keyed.column.as_str());

		for foreign_key in &table.foreign_keys {
			let [column] = foreign_key.columns.as_slice() else {
				continue;
			};

			if Some(column.as_str()) == key || !targets.contains(&foreign_key.references) {
				continue;
			}

			let reference = AccountReference {
				table: table.name.clone(),
				column: column.clone(),
				references: foreign_key.references.clone(),
				strategy: table.on_account_merge.unwrap_or(MergeStrategy::Reassign),
				explicit: table.on_account_merge.is_some(),
				unique_with: unique_with(table, column),
				has_updated_at: table.column("updated_at").is_some(),
			};

			if !reference.explicit && !reference.unique_with.is_empty() {
				data.problems.push(MergeProblem::NeedsStrategy {
					table: table.name.clone(),
					column: column.clone(),
				});
			}

			if reference.strategy == MergeStrategy::KeepNewest
				&& !(reference.one_per_account() && reference.has_updated_at)
			{
				data.problems.push(MergeProblem::KeepNewestNeedsOneRow {
					table: table.name.clone(),
				});
			}

			data.references.push(reference);
		}
	}

	data
}

/// The unique constraints `column` is part of, each as its other columns.
fn unique_with(table: &Table, column: &str) -> Vec<Vec<String>> {
	let others = |columns: &[String]| -> Option<Vec<String>> {
		columns.iter().any(|name| name == column).then(|| {
			columns
				.iter()
				.filter(|name| *name != column)
				.cloned()
				.collect()
		})
	};

	others(&table.primary_key)
		.into_iter()
		.chain(
			table
				.indexes
				.iter()
				.filter(|index| index.unique)
				.filter_map(|index| others(&index.columns)),
		)
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Column;
	use crate::ColumnType;
	use crate::Dialect;
	use crate::ForeignKey;
	use crate::Index;
	use crate::ReferentialAction;

	fn table(name: TableName, columns: &[&str], key: &[&str]) -> Table {
		Table::new(
			name,
			columns
				.iter()
				.map(|column| Column::new(*column, ColumnType::Uuid))
				.collect(),
			key.iter().map(ToString::to_string).collect(),
		)
	}

	fn link(table: &mut Table, column: &str, to: &TableName) {
		table.foreign_keys.push(
			ForeignKey::new(&table.name, &[column], to.clone(), &["id"])
				.on_delete(ReferentialAction::Cascade),
		);
	}

	fn schema() -> (Schema, TableName) {
		let accounts = TableName::namespaced("auth", "accounts");
		let sessions = TableName::namespaced("auth", "sessions");
		let users = TableName::new("users");
		let profiles = TableName::new("profiles");
		let posts = TableName::new("posts");
		let follows = TableName::new("follows");
		let settings = TableName::new("settings");
		let tags = TableName::new("tags");

		let mut session = table(sessions, &["id", "account_id"], &["id"]);
		link(&mut session, "account_id", &accounts);
		let mut user = table(users.clone(), &["id", "invited_by"], &["id"]);
		link(&mut user, "id", &accounts);
		link(&mut user, "invited_by", &users);
		let mut profile = table(profiles, &["id"], &["id"]);
		link(&mut profile, "id", &users);
		let mut post = table(posts, &["id", "author_id"], &["id"]);

		link(&mut post, "author_id", &users);
		let mut follow = table(
			follows.clone(),
			&["follower_id", "followee_id"],
			&["follower_id", "followee_id"],
		);
		link(&mut follow, "follower_id", &users);
		link(&mut follow, "followee_id", &users);
		let mut setting = table(settings.clone(), &["id", "user_id", "updated_at"], &["id"]);
		setting.indexes.push(Index::unique(&settings, &["user_id"]));
		link(&mut setting, "user_id", &users);
		let tag = table(tags, &["id"], &["id"]);

		let schema = Schema::new(
			Dialect::Postgres,
			vec![
				table(accounts.clone(), &["id"], &["id"]),
				session,
				user,
				profile,
				post,
				follow,
				setting,
				tag,
			],
		);
		(schema, accounts)
	}

	#[test]
	fn keyed_tables_and_references_are_found() {
		let (schema, accounts) = schema();
		let data = account_data(&schema, &accounts);

		let keyed: Vec<String> = data
			.keyed
			.iter()
			.map(|keyed| keyed.table.to_string())
			.collect();
		assert_eq!(keyed, ["users", "profiles"]);
		let references: Vec<String> = data
			.references
			.iter()
			.map(|reference| format!("{}.{}", reference.table, reference.column))
			.collect();
		assert_eq!(
			references,
			[
				"follows.follower_id",
				"follows.followee_id",
				"posts.author_id",
				"settings.user_id",
				"users.invited_by",
			]
		);
		// The pack's own tables aren't the app's data.
		assert!(!references.iter().any(|name| name.starts_with("auth")));
		assert_eq!(data.references[2].unique_with, [] as [Vec<String>; 0]);
		assert_eq!(
			data.references[0].unique_with,
			[vec!["followee_id".to_owned()]]
		);
		assert!(data.references[3].one_per_account());

		assert!(!data.references[0].one_per_account());
	}

	#[test]
	fn unique_references_need_a_strategy() {
		let (mut schema, accounts) = schema();
		let problems: Vec<String> = account_data(&schema, &accounts)
			.problems
			.iter()
			.map(ToString::to_string)
			.collect();
		assert_eq!(problems.len(), 3, "{problems:#?}");
		assert!(problems[0].starts_with("follows.follower_id is in a unique constraint"));
		assert!(!account_data(&schema, &accounts).is_covered());

		for table in &mut schema.tables {
			match table.name.name.as_str() {
				"follows" => table.on_account_merge = Some(MergeStrategy::ReassignSkipConflicts),
				"settings" => table.on_account_merge = Some(MergeStrategy::KeepNewest),
				_ => {}
			}
		}

		let data = account_data(&schema, &accounts);
		assert!(data.is_covered(), "{:#?}", data.problems);
		assert!(!data.has_custom());
	}

	#[test]
	fn strategies_must_fit_their_table() {
		let (mut schema, accounts) = schema();

		for table in &mut schema.tables {
			match table.name.name.as_str() {
				"follows" | "posts" => table.on_account_merge = Some(MergeStrategy::KeepNewest),
				"settings" | "profiles" => table.on_account_merge = Some(MergeStrategy::Custom),
				"users" => table.on_account_merge = Some(MergeStrategy::Reassign),
				_ => {}
			}
		}

		let data = account_data(&schema, &accounts);
		let problems: Vec<String> = data.problems.iter().map(ToString::to_string).collect();
		assert_eq!(
			problems,
			[
				"users is keyed by the account, so it keeps the survivor's row: use \
				 `keep_survivor` or `custom`, not `reassign`",
				"follows uses `keep_newest`, which needs one row per account (the reference alone \
				 unique) and an `updated_at` column",
				"follows uses `keep_newest`, which needs one row per account (the reference alone \
				 unique) and an `updated_at` column",
				"posts uses `keep_newest`, which needs one row per account (the reference alone \
				 unique) and an `updated_at` column",
			]
		);
		assert!(data.has_custom());
	}

	#[test]
	fn strategies_never_reach_snapshots() {
		let (mut schema, _) = schema();
		schema.tables[2].on_account_merge = Some(MergeStrategy::Custom);
		let snapshot = schema.without_rename_hints();
		assert!(
			snapshot
				.tables
				.iter()
				.all(|table| table.on_account_merge.is_none())
		);
		assert!(!snapshot.to_json().contains("on_account_merge"));
		assert!(
			schema
				.to_json()
				.contains("\"on_account_merge\": \"custom\"")
		);
		assert_eq!(
			MergeStrategy::ReassignSkipConflicts.as_str(),
			"reassign_skip_conflicts"
		);
	}
}
