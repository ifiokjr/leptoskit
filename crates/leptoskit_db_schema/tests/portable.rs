//! A schema pack's tables must work on every dialect: what passes, and how
//! each kind of non-portable table is reported.

use leptoskit_db_schema::Check;
use leptoskit_db_schema::CheckRule;
use leptoskit_db_schema::Column;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::DefaultValue;
use leptoskit_db_schema::ForeignKey;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::SchemaError;
use leptoskit_db_schema::Table;
use leptoskit_db_schema::TableName;

fn table(name: &str, columns: Vec<Column>) -> Table {
	let mut all = vec![Column::new("id", ColumnType::Uuid)];
	all.extend(columns);
	Table::new(TableName::namespaced("auth", name), all, vec!["id".into()])
}

fn problems(tables: &[Table]) -> Vec<String> {
	match Schema::validate_portable(tables) {
		Ok(()) => Vec::new(),
		Err(SchemaError::Invalid(problems)) => problems.iter().map(ToString::to_string).collect(),
		Err(other) => panic!("{other}"),
	}
}

#[test]
fn portable_tables_pass() {
	let sessions = table(
		"sessions",
		vec![
			Column::new("user_agent", ColumnType::TEXT).nullable(),
			Column::new("active", ColumnType::Bool).default(DefaultValue::Bool(true)),
		],
	);
	assert_eq!(problems(&[sessions]), [] as [String; 0]);
}

#[test]
fn non_portable_tables_are_explained() {
	let mut accounts = table(
		"accounts",
		vec![
			Column::new(
				"tags",
				ColumnType::Array {
					element: Box::new(ColumnType::TEXT),
				},
			),
			Column::new("joined", ColumnType::Timestamp).default(DefaultValue::Sql("now()".into())),
		],
	);
	accounts.checks.push(Check {
		name: "auth_accounts_tags_check".into(),
		rule: CheckRule::Sql {
			sql: "cardinality(tags) < 10".into(),
		},
	});
	// A pack can't link to a table outside it.
	accounts.foreign_keys.push(ForeignKey::new(
		&accounts.name,
		&["id"],
		TableName::new("users"),
		&["id"],
	));

	insta::assert_snapshot!(problems(&[accounts]).join("\n"));
}
