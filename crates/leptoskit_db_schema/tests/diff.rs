//! How the diff matches tables and columns, judges risk, and refuses what it
//! can't do safely.

mod support;

use leptoskit_db_schema::*;
use support::blog;
use support::column_mut;
use support::table_mut;

fn renamed_handle(dialect: Dialect) -> Schema {
	let mut schema = blog(dialect);
	let users = table_mut(&mut schema, "users");
	users.indexes = vec![Index::unique(&TableName::new("users"), &["username"])];
	column_mut(users, "handle").name = "username".into();
	schema
}

#[test]
fn identical_schemas_need_no_steps() {
	let schema = blog(Dialect::Postgres);
	let plan = diff(&schema, &schema, &mut RequireHints).unwrap();
	assert!(plan.is_empty());
}

#[test]
fn an_unhinted_rename_is_ambiguous_where_nobody_can_be_asked() {
	let error = diff(
		&blog(Dialect::Postgres),
		&renamed_handle(Dialect::Postgres),
		&mut RequireHints,
	)
	.unwrap_err();
	insta::assert_snapshot!(error.to_string());
}

#[test]
fn a_resolver_can_answer_an_ambiguous_rename() {
	struct Yes;

	impl RenameResolver for Yes {
		fn table(&mut self, _: &Table, _: &[&Table]) -> Result<Option<TableName>, DiffError> {
			Ok(None)
		}

		fn column(
			&mut self,
			_: &TableName,
			_: &Column,
			candidates: &[&Column],
		) -> Result<Option<String>, DiffError> {
			Ok(Some(candidates[0].name.clone()))
		}
	}

	let plan = diff(
		&blog(Dialect::Postgres),
		&renamed_handle(Dialect::Postgres),
		&mut Yes,
	)
	.unwrap();
	assert!(plan.steps.contains(&Step::RenameColumn {
		table: TableName::new("users"),
		from: "handle".into(),
		to: "username".into(),
	}));
	assert_eq!(plan.destructive_steps().count(), 0);
}

#[test]
fn without_renames_a_replaced_column_loses_data() {
	let mut target = renamed_handle(Dialect::Postgres);
	column_mut(table_mut(&mut target, "users"), "username").nullable = true;

	let plan = diff(&blog(Dialect::Postgres), &target, &mut NoRenames).unwrap();
	let destructive: Vec<String> = plan.destructive_steps().map(ToString::to_string).collect();
	assert_eq!(destructive, ["- column users.handle"]);
}

#[test]
fn a_hint_that_already_applied_is_ignored() {
	let mut target = renamed_handle(Dialect::Postgres);
	column_mut(table_mut(&mut target, "users"), "username").renamed_from = Some("handle".into());
	let snapshot = target.without_rename_hints();

	let plan = diff(&snapshot, &target, &mut RequireHints).unwrap();
	assert!(plan.is_empty(), "{:?}", plan.steps);
}

#[test]
fn a_hint_naming_nothing_is_an_error() {
	let mut target = renamed_handle(Dialect::Postgres);
	column_mut(table_mut(&mut target, "users"), "username").renamed_from = Some("nickname".into());

	let error = diff(&blog(Dialect::Postgres), &target, &mut RequireHints).unwrap_err();
	insta::assert_snapshot!(error.to_string());
}

#[test]
fn a_required_column_needs_a_default_on_an_existing_table() {
	let mut target = blog(Dialect::Postgres);
	table_mut(&mut target, "users")
		.columns
		.push(Column::new("country", ColumnType::TEXT));

	let error = diff(&blog(Dialect::Postgres), &target, &mut RequireHints).unwrap_err();
	insta::assert_snapshot!(error.to_string());
}

#[test]
fn renaming_a_referenced_table_leaves_foreign_keys_alone() {
	let mut target = blog(Dialect::Sqlite);
	let users = table_mut(&mut target, "users");
	users.name = TableName::new("members");
	users.renamed_from = Some(TableName::new("users"));
	users.indexes = vec![Index::unique(&TableName::new("members"), &["handle"])];
	table_mut(&mut target, "posts").foreign_keys[0].references = TableName::new("members");
	let target = Schema::new(Dialect::Sqlite, target.tables);

	let plan = diff(&blog(Dialect::Sqlite), &target, &mut RequireHints).unwrap();
	assert!(
		plan.steps
			.iter()
			.all(|step| step.table() == &TableName::new("members")),
		"only the renamed table changes: {:?}",
		plan.steps
	);
}

#[test]
fn retyping_a_referenced_column_rebuilds_its_foreign_keys() {
	let mut target = blog(Dialect::Postgres);
	column_mut(table_mut(&mut target, "users"), "id").ty = ColumnType::Int32;
	column_mut(table_mut(&mut target, "posts"), "author_id").ty = ColumnType::Int32;

	let plan = diff(&blog(Dialect::Postgres), &target, &mut RequireHints).unwrap();
	let summary: Vec<String> = plan.steps.iter().map(ToString::to_string).collect();
	insta::assert_snapshot!(summary.join("\n"));
}

#[test]
fn risks_follow_what_happens_to_existing_rows() {
	let users = TableName::new("users");
	let column = |ty| Column::new("value", ty);
	let alter = |from: Column, to: Column| {
		Step::AlterColumn {
			table: users.clone(),
			from,
			to,
		}
	};

	assert_eq!(
		alter(column(ColumnType::Int32), column(ColumnType::Int64)).risk(),
		Risk::Safe
	);
	assert_eq!(
		alter(
			column(ColumnType::Text { max_len: Some(10) }),
			column(ColumnType::TEXT)
		)
		.risk(),
		Risk::Safe
	);
	assert!(matches!(
		alter(column(ColumnType::Int64), column(ColumnType::Int32)).risk(),
		Risk::LosesData(_)
	));
	assert!(matches!(
		alter(
			column(ColumnType::TEXT),
			column(ColumnType::Text { max_len: Some(10) })
		)
		.risk(),
		Risk::LosesData(_)
	));
	assert!(matches!(
		alter(
			column(ColumnType::TEXT).nullable(),
			column(ColumnType::TEXT)
		)
		.risk(),
		Risk::MayFail(_)
	));
	assert_eq!(
		alter(
			column(ColumnType::TEXT).nullable(),
			column(ColumnType::TEXT).default(DefaultValue::Text(String::new()))
		)
		.risk(),
		Risk::Safe
	);
}

#[test]
fn schemas_must_be_for_one_dialect() {
	let error = diff(
		&blog(Dialect::Postgres),
		&blog(Dialect::Sqlite),
		&mut RequireHints,
	)
	.unwrap_err();
	assert!(matches!(error, DiffError::DialectMismatch { .. }));
}

#[test]
fn validation_reports_every_problem_with_its_location() {
	let mut schema = blog(Dialect::Sqlite);
	let posts = table_mut(&mut schema, "posts");
	posts.columns.push(Column::new(
		"Tags",
		ColumnType::Array {
			element: Box::new(ColumnType::TEXT),
		},
	));
	column_mut(posts, "author_id").ty = ColumnType::Int32;
	posts.primary_key = vec!["missing".into()];
	posts.checks.push(Check {
		name: "posts_title_range".into(),
		rule: CheckRule::Range {
			column: "title".into(),
			min: Some(0.0),
			max: None,
		},
	});
	table_mut(&mut schema, "users").columns.push(
		Column::new("nickname", ColumnType::Text { max_len: Some(3) })
			.default(DefaultValue::Text("toolong".into())),
	);

	insta::assert_snapshot!(schema.validate().unwrap_err().to_string());
}

#[test]
fn snapshots_round_trip_without_hints() {
	let mut schema = renamed_handle(Dialect::Postgres);
	column_mut(table_mut(&mut schema, "users"), "username").renamed_from = Some("handle".into());

	let json = schema.without_rename_hints().to_json();
	assert!(!json.contains("renamed_from"));
	assert_eq!(
		Schema::from_json(&json).unwrap(),
		schema.without_rename_hints()
	);
	insta::assert_snapshot!(json);
}

#[test]
fn snapshots_from_another_format_are_refused() {
	let json = Schema::empty(Dialect::Sqlite)
		.to_json()
		.replace("\"format\": 1", "\"format\": 2");
	let error = Schema::from_json(&json).unwrap_err();
	assert!(matches!(
		error,
		SchemaError::UnsupportedFormat {
			found: 2,
			supported: 1
		}
	));
}

#[test]
fn long_names_stay_unique_within_postgres_limits() {
	let table = TableName::namespaced("billing", "subscription_invoice_line_items");
	let one = constraint_name(
		&table,
		&["subscription_invoice_id", "line_item_position"],
		"key",
	);
	let two = constraint_name(
		&table,
		&["subscription_invoice_id", "line_item_position_alt"],
		"key",
	);

	assert!(one.len() <= MAX_IDENTIFIER_LEN && two.len() <= MAX_IDENTIFIER_LEN);
	assert_ne!(one, two);
	assert_eq!(
		constraint_name(&TableName::new("users"), &["handle"], "key"),
		"users_handle_key"
	);
}

fn renamed_users_table(dialect: Dialect) -> Schema {
	let mut schema = blog(dialect);
	let members = TableName::new("members");
	let users = table_mut(&mut schema, "users");
	users.name = members.clone();
	users.indexes = vec![Index::unique(&members, &["handle"])];
	table_mut(&mut schema, "posts").foreign_keys[0].references = members;
	Schema::new(dialect, schema.tables)
}

/// Answers every ambiguity with the first candidate, or with a fixed name.
struct Answer(Option<&'static str>);

impl RenameResolver for Answer {
	fn table(&mut self, _: &Table, candidates: &[&Table]) -> Result<Option<TableName>, DiffError> {
		Ok(Some(self.0.map_or_else(
			|| candidates[0].name.clone(),
			TableName::new,
		)))
	}

	fn column(
		&mut self,
		_: &TableName,
		_: &Column,
		candidates: &[&Column],
	) -> Result<Option<String>, DiffError> {
		Ok(Some(self.0.map_or_else(
			|| candidates[0].name.clone(),
			ToString::to_string,
		)))
	}
}

#[test]
fn an_unhinted_table_rename_is_ambiguous_where_nobody_can_be_asked() {
	let error = diff(
		&blog(Dialect::Sqlite),
		&renamed_users_table(Dialect::Sqlite),
		&mut RequireHints,
	)
	.unwrap_err();
	insta::assert_snapshot!(error.to_string());
}

#[test]
fn a_resolver_can_answer_an_ambiguous_table_rename() {
	let plan = diff(
		&blog(Dialect::Sqlite),
		&renamed_users_table(Dialect::Sqlite),
		&mut Answer(None),
	)
	.unwrap();
	assert!(plan.steps.contains(&Step::RenameTable {
		from: TableName::new("users"),
		to: TableName::new("members")
	}));
	assert_eq!(plan.destructive_steps().count(), 0);
}

#[test]
fn without_renames_a_replaced_table_loses_data() {
	let plan = diff(
		&blog(Dialect::Sqlite),
		&renamed_users_table(Dialect::Sqlite),
		&mut NoRenames,
	)
	.unwrap();
	let destructive: Vec<String> = plan.destructive_steps().map(ToString::to_string).collect();
	assert_eq!(destructive, ["- table users"]);
}

#[test]
fn a_resolver_must_answer_with_a_candidate() {
	let table = diff(
		&blog(Dialect::Sqlite),
		&renamed_users_table(Dialect::Sqlite),
		&mut Answer(Some("nothing")),
	);
	let column = diff(
		&blog(Dialect::Postgres),
		&renamed_handle(Dialect::Postgres),
		&mut Answer(Some("nothing")),
	);
	insta::assert_snapshot!(format!("{}\n{}", table.unwrap_err(), column.unwrap_err()));
}

#[test]
fn a_table_hint_naming_nothing_is_an_error() {
	let mut target = renamed_users_table(Dialect::Sqlite);
	table_mut(&mut target, "members").renamed_from = Some(TableName::new("people"));

	let error = diff(&blog(Dialect::Sqlite), &target, &mut NoRenames).unwrap_err();
	insta::assert_snapshot!(error.to_string());
}

#[test]
fn column_types_read_as_written() {
	let types = [
		ColumnType::Bool,
		ColumnType::Int16,
		ColumnType::Float32,
		ColumnType::Float64,
		ColumnType::Decimal {
			precision: Some(10),
			scale: Some(2),
		},
		ColumnType::Decimal {
			precision: Some(10),
			scale: None,
		},
		ColumnType::Decimal {
			precision: None,
			scale: None,
		},
		ColumnType::Bytes,
		ColumnType::Uuid,
		ColumnType::Date,
		ColumnType::Json,
		ColumnType::Array {
			element: Box::new(ColumnType::Int64),
		},
	];
	let written: Vec<String> = types.iter().map(ToString::to_string).collect();
	assert_eq!(
		written,
		[
			"bool",
			"int16",
			"float32",
			"float64",
			"decimal(10, 2)",
			"decimal(10)",
			"decimal",
			"bytes",
			"uuid",
			"date",
			"json",
			"int64[]"
		]
	);
	assert_eq!(
		[ReferentialAction::NoAction, ReferentialAction::Restrict].map(ReferentialAction::sql),
		["NO ACTION", "RESTRICT"]
	);
}

#[test]
fn validation_checks_keys_indexes_and_references() {
	let users = TableName::new("users");
	let mut schema = blog(Dialect::Postgres);
	let mut twin = Table::new(TableName::namespaced("users", "twin"), vec![], vec![]);
	twin.indexes.push(Index {
		name: "users_handle_key".into(),
		columns: vec![],
		unique: false,
		predicate: None,
	});
	schema.tables.push(twin);
	schema.tables.push(Table::new(
		TableName::new("users_twin"),
		vec![Column::new("id", ColumnType::Int64).nullable()],
		vec!["id".into()],
	));

	let posts = table_mut(&mut schema, "posts");
	posts.indexes.push(
		Index::new(&TableName::new("posts"), &["missing"])
			.partial(Predicate::IsNull("also_missing".into())),
	);
	posts.foreign_keys.push(ForeignKey::new(
		&TableName::new("posts"),
		&["author_id"],
		users.clone(),
		&["id", "handle"],
	));
	posts.foreign_keys.push(
		ForeignKey::new(
			&TableName::new("posts"),
			&["title"],
			TableName::new("nowhere"),
			&["id"],
		)
		.on_delete(ReferentialAction::SetNull),
	);
	let mut by_title = ForeignKey::new(
		&TableName::new("posts"),
		&["author_id"],
		users,
		&["missing"],
	);
	by_title.name = "posts_by_title_fkey".into();
	posts.foreign_keys.push(by_title);
	let mut not_a_key = ForeignKey::new(
		&TableName::new("posts"),
		&["title"],
		TableName::new("users"),
		&["display_name"],
	)
	.on_delete(ReferentialAction::SetNull);
	not_a_key.name = "posts_title_display_fkey".into();
	posts.foreign_keys.push(not_a_key);
	posts.checks.push(Check {
		name: "posts_ghost_check".into(),
		rule: CheckRule::Sql { sql: "TRUE".into() },
	});
	posts.checks.push(Check {
		name: "posts_ghost_check".into(),
		rule: CheckRule::Length {
			column: "ghost".into(),
			min: None,
			max: Some(3),
		},
	});
	posts.columns.push(Column::new(
		"nested",
		ColumnType::Array {
			element: Box::new(ColumnType::Array {
				element: Box::new(ColumnType::Int64),
			}),
		},
	));
	posts
		.columns
		.push(Column::new("flag", ColumnType::Bool).default(DefaultValue::Integer(1)));
	posts
		.columns
		.push(Column::new("at", ColumnType::Date).default(DefaultValue::Now));
	posts
		.columns
		.push(Column::new("ratio", ColumnType::Float64).default(DefaultValue::Float(f64::NAN)));
	posts
		.columns
		.push(Column::new("label", ColumnType::Int64).default(DefaultValue::Text("x".into())));
	posts
		.columns
		.push(Column::new("on", ColumnType::TEXT).default(DefaultValue::Bool(true)));
	posts.columns.push(Column::new("score", ColumnType::Int32));
	posts.columns.push(
		Column::new("x".repeat(64), ColumnType::Int32).default(DefaultValue::Sql("1".into())),
	);

	insta::assert_snapshot!(schema.validate().unwrap_err().to_string());
}

#[test]
fn every_scenario_schema_round_trips_through_json() {
	for scenario in support::scenarios() {
		for dialect in [Dialect::Postgres, Dialect::Sqlite] {
			if scenario.only.is_some_and(|only| only != dialect) {
				continue;
			}

			for schema in [(scenario.before)(dialect), (scenario.after)(dialect)] {
				let snapshot = schema.without_rename_hints();
				let json = snapshot.to_json();
				assert_eq!(
					Schema::from_json(&json).unwrap(),
					snapshot,
					"{} ({dialect})",
					scenario.name
				);
			}
		}
	}
}
