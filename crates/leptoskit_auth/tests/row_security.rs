//! The auth tables' policies read the signed-in account as a setting, so
//! an app with `[db] policies = "rls"` gets them as Postgres row-level
//! security too.

#![cfg(feature = "server")]

use leptoskit_auth as _;

#[test]
fn the_auth_tables_export_as_row_level_security() {
	let schema = leptoskit_db::schema_with_row_security(leptoskit_db::Dialect::Postgres).unwrap();
	let using = |table: &str| {
		schema
			.tables
			.iter()
			.find(|candidate| candidate.name.name == table)
			.and_then(|table| table.row_security.as_ref())
			.and_then(|security| security.policies.first())
			.and_then(|select| select.using.clone())
			.unwrap_or_else(|| panic!("`{table}` has a select policy"))
	};

	assert_eq!(
		using("accounts"),
		"\"id\" = NULLIF(current_setting('leptoskit.account', true), '')::uuid"
	);
	assert!(using("sessions").contains("\"account_id\" = NULLIF"));
	// Tables nobody reads directly stay closed to raw SQL too.
	assert!(
		schema
			.tables
			.iter()
			.filter_map(|table| table.row_security.as_ref())
			.any(|security| security.policies[0].using.as_deref() == Some("1 = 0"))
	);
}
