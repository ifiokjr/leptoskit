//! The provider's tables read the signed-in account as a setting, as the
//! auth tables do, so an app with `[db] policies = "rls"` gets them as
//! Postgres row-level security too.

#![cfg(feature = "server")]

use leptoskit_idp as _;

#[test]
fn the_provider_tables_export_as_row_level_security() {
	let schema = leptoskit_db::schema_with_row_security(leptoskit_db::Dialect::Postgres).unwrap();
	let using = |table: &str| {
		schema
			.tables
			.iter()
			.find(|candidate| {
				candidate.name.namespace.as_deref() == Some("idp") && candidate.name.name == table
			})
			.and_then(|table| table.row_security.as_ref())
			.and_then(|security| security.policies.first())
			.and_then(|select| select.using.clone())
			.unwrap_or_else(|| panic!("`idp.{table}` has a select policy"))
	};

	for table in ["grants", "families"] {
		assert_eq!(
			using(table),
			"\"account_id\" = NULLIF(current_setting('leptoskit.account', true), '')::uuid",
			"idp.{table}"
		);
	}

	assert_eq!(using("clients"), "1 = 0");
}
