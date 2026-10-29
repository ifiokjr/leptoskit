//! `leptoskit::auth` re-exports sign-in: an app's table links to the pack's
//! accounts and reads the signed-in person in its policies, through the
//! facade alone.

#![cfg(all(feature = "auth", feature = "db-sqlite"))]

use leptoskit::auth;
use leptoskit::auth::AuthContext;
use leptoskit::db;
use leptoskit::db::Id;
use leptoskit::db::Table;

/// The app's profile, keyed by the account it belongs to.
#[db::table(name = "profiles", crate = "leptoskit::db")]
#[db::policy(all, |profile, cx| cx.signed_in(|me| profile.id.eq(me.account)))]
pub struct Profile {
	#[db(on_delete = cascade)]
	pub id: Id<auth::Account>,
	pub bio: String,
}

#[test]
fn an_app_table_links_to_the_auth_pack() {
	let schema = db::schema(db::Dialect::Sqlite).unwrap();
	let table = |name: &str| schema.tables.iter().find(|table| table.name.flat() == name);
	let profiles = table("profiles").unwrap();

	assert_eq!(Profile::policies().len(), 1);
	assert_eq!(profiles.foreign_keys[0].references.flat(), "auth_accounts");
	assert!(table("auth_sessions").is_some());
}
