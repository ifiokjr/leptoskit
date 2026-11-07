//! A consumer that only names the pack in `packs!`: nothing else in the
//! program mentions the fixture's types, so this is the case where an
//! earlier `extern crate … as _` could let every pack table fall out of the
//! link.

leptoskit_db::packs!(leptoskit_db_pack_fixture);

use leptoskit_db::Dialect;
use leptoskit_db_schema::TableName;

#[test]
fn only_the_pack_ref_brings_every_table_and_its_membership() {
	for dialect in [Dialect::Postgres, Dialect::Sqlite] {
		let schema = leptoskit_db::schema(dialect).unwrap();
		let sample = schema
			.pack("sample")
			.unwrap_or_else(|| panic!("{dialect}: the sample pack is missing"));

		assert!(!sample.builtin, "{dialect}");
		assert_eq!(
			sample.tables,
			[
				TableName::namespaced("sample", "notes"),
				TableName::namespaced("sample", "related_notes"),
				TableName::namespaced("sample", "signatures"),
			],
			"{dialect}"
		);

		for table in &sample.tables {
			assert!(schema.table(table).is_some(), "{dialect}: {table}");
		}
	}
}
