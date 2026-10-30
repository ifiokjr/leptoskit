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

#[test]
fn a_generated_column_is_added_for_existing_rows_and_dropped_safely() {
	let before = blog(Dialect::Postgres);
	let after = support::with_generated(Dialect::Postgres);

	// NOT NULL without a default is fine: the database computes it.
	let added = diff(&before, &after, &mut RequireHints).unwrap();
	assert!(added.steps.iter().any(|step| {
		matches!(step, Step::AddColumn { column, .. } if column.name == "handle_key")
	}));

	// Dropping it loses nothing the database couldn't recompute.
	let dropped = diff(&after, &before, &mut RequireHints).unwrap();
	assert_eq!(dropped.destructive_steps().count(), 0);
	assert!(dropped.steps.iter().all(|step| step.risk() == Risk::Safe));
}

#[test]
fn changing_how_a_column_is_generated_is_refused() {
	let before = support::with_generated(Dialect::Sqlite);
	let message = |after: &Schema| {
		diff(&before, after, &mut RequireHints)
			.unwrap_err()
			.to_string()
	};

	let mut changed = before.clone();
	let users = table_mut(&mut changed, "users");
	column_mut(users, "handle_key").generated = Some(Generated::new("upper(\"handle\")"));
	assert_eq!(
		message(&changed),
		"users.handle_key changes how it's generated, or whether it is; add a new column under \
		 another name and drop this one, so the migration rebuilds its indexes too"
	);

	let mut written = before.clone();
	column_mut(table_mut(&mut written, "users"), "handle_key").generated = None;
	assert!(message(&written).starts_with("users.handle_key changes how it's generated"));

	let mut generated = before.clone();
	column_mut(table_mut(&mut generated, "users"), "display_name").generated =
		Some(Generated::new("\"handle\""));
	assert!(message(&generated).starts_with("users.display_name changes"));

	// Only the project's own dialect counts: a Postgres expression changing
	// in a SQLite project changes nothing.
	let mut other_dialect = before.clone();
	column_mut(table_mut(&mut other_dialect, "posts"), "title_length")
		.generated
		.as_mut()
		.unwrap()
		.postgres = "length(\"title\")".into();
	assert!(
		diff(&before, &other_dialect, &mut RequireHints)
			.unwrap()
			.is_empty()
	);
}

#[test]
fn a_generated_column_is_never_a_key_a_link_or_a_default() {
	let mut schema = support::with_generated(Dialect::Postgres);
	let users_name = TableName::new("users");
	let users = table_mut(&mut schema, "users");
	column_mut(users, "handle_key").default = Some(DefaultValue::Text("x".into()));
	users.primary_key = vec!["handle_key".into()];
	users.foreign_keys.push(ForeignKey::new(
		&users_name,
		&["handle_key"],
		users_name.clone(),
		&["handle_key"],
	));
	column_mut(users, "display_name").generated = Some(Generated {
		postgres: " ".into(),
		sqlite: "x".into(),
	});

	let SchemaError::Invalid(problems) = schema.validate().unwrap_err() else {
		panic!("the schema should be invalid");
	};
	let messages: Vec<String> = problems.iter().map(ToString::to_string).collect();
	for expected in [
		"users.handle_key: a generated column can't have a default: the database computes every \
		 value",
		"users: the primary key column `handle_key` can't be generated",
		"users: foreign key `users_handle_key_fkey` is on `handle_key`, a generated column; link \
		 from a column the app writes",
		"users.display_name: the generated column has no expression on postgres",
	] {
		assert!(
			messages.iter().any(|message| message == expected),
			"{expected}\n{messages:#?}"
		);
	}
}

#[test]
fn generated_columns_round_trip_through_snapshots() {
	let schema = support::with_generated(Dialect::Sqlite);
	let json = schema.to_json();
	assert!(
		json.contains("\"sqlite\": \"length(\\\"title\\\")\""),
		"{json}"
	);
	assert_eq!(Schema::from_json(&json).unwrap(), schema);

	// Snapshots written before generated columns existed still read.
	let older = blog(Dialect::Sqlite).to_json();
	assert!(!older.contains("generated"));
	assert_eq!(Schema::from_json(&older).unwrap(), blog(Dialect::Sqlite));
}

#[test]
fn views_are_validated_with_the_tables() {
	let view = |name: TableName, columns: Vec<Column>, sql: &str| {
		View::new(name, columns, Generated::new(sql))
	};
	let schema = blog(Dialect::Sqlite).with_views(vec![
		view(
			TableName::new("posts"),
			vec![Column::new("id", ColumnType::Int64)],
			"SELECT 1",
		),
		view(TableName::new("empty"), Vec::new(), "SELECT 1"),
		view(
			TableName::new("doubled"),
			vec![
				Column::new("id", ColumnType::Int64),
				Column::new("id", ColumnType::Int64).default(DefaultValue::Integer(1)),
				Column::new(
					"tags",
					ColumnType::Array {
						element: Box::new(ColumnType::TEXT),
					},
				),
			],
			"  ",
		),
		view(
			TableName::namespaced("Bad Schema", "x"),
			vec![Column::new("Bad Name", ColumnType::Int64)],
			"SELECT 1",
		),
	]);

	insta::assert_snapshot!(schema.validate().unwrap_err().to_string());
}

#[test]
fn views_round_trip_through_snapshots_and_change_only_when_they_differ() {
	let view = View::new(
		TableName::new("titles"),
		vec![Column::new("title", ColumnType::TEXT)],
		Generated {
			postgres: "SELECT title FROM posts".into(),
			sqlite: "SELECT title FROM posts".into(),
		},
	);
	let schema = blog(Dialect::Postgres).with_views(vec![view.clone()]);
	let json = schema.to_json();
	assert!(!json.contains("materialized"), "{json}");
	assert!(!json.contains("comment\": null"), "{json}");
	assert_eq!(Schema::from_json(&json).unwrap(), schema);
	assert_eq!(schema.view(&TableName::new("titles")), Some(&view));
	assert!(
		diff(&schema, &schema, &mut RequireHints)
			.unwrap()
			.is_empty()
	);

	// The other dialect's query doesn't matter to this one.
	let mut sqlite_only = view.clone();
	sqlite_only.sql.sqlite = "SELECT upper(title) FROM posts".into();
	assert!(view.same_on(&sqlite_only, Dialect::Postgres));
	assert!(!view.same_on(&sqlite_only, Dialect::Sqlite));
	let other = blog(Dialect::Postgres).with_views(vec![sqlite_only]);
	assert!(diff(&schema, &other, &mut RequireHints).unwrap().is_empty());

	// A new comment, or materializing it, recreates it.
	for change in [
		|view: &mut View| view.comment = Some("Titles.".into()),
		|view: &mut View| view.materialized = true,
	] {
		let mut changed = view.clone();
		change(&mut changed);
		let target = blog(Dialect::Postgres).with_views(vec![changed.clone()]);
		let plan = diff(&schema, &target, &mut RequireHints).unwrap();
		assert_eq!(
			plan.steps,
			[Step::DropView(view.clone()), Step::CreateView(changed)]
		);
		assert!(plan.steps.iter().all(|step| step.risk() == Risk::Safe));
		assert_eq!(plan.steps[0].table(), &TableName::new("titles"));
	}

	// Adding a table or an index leaves views alone.
	let mut grown = schema.clone();
	grown.tables.push(Table::new(
		TableName::new("zines"),
		vec![Column::new("id", ColumnType::Int64)],
		vec!["id".into()],
	));
	let plan = diff(&schema, &grown, &mut RequireHints).unwrap();
	assert!(
		matches!(plan.steps.as_slice(), [Step::CreateTable(_)]),
		"{:?}",
		plan.steps
	);
}

#[test]
fn views_know_which_views_their_queries_name() {
	let view = |name: TableName, sql: &str| {
		View::new(
			name,
			vec![Column::new("id", ColumnType::Int64)],
			Generated::new(sql),
		)
	};
	let totals = TableName::new("totals");
	let reports = TableName::namespaced("reports", "totals");
	let reads = |sql: &str, other: &TableName| {
		view(TableName::new("reader"), sql).may_read(other, Dialect::Postgres)
	};

	assert!(reads("SELECT id FROM totals", &totals));
	assert!(reads("select id from \"totals\" t", &totals));
	assert!(reads("SELECT id FROM TOTALS", &totals));
	assert!(reads("SELECT id FROM reports.totals", &reports));
	assert!(reads("SELECT id FROM reports_totals", &reports));
	assert!(!reads("SELECT 'totals' AS id FROM posts", &totals));
	assert!(!reads("SELECT id FROM posts -- not totals\n", &totals));
	assert!(!reads("SELECT id FROM subtotals", &totals));
	assert!(!reads("SELECT 'it''s totals' FROM posts", &totals));

	// Ordered by what they read, then by name; a cycle has no order.
	let schema = blog(Dialect::Postgres).with_views(vec![
		view(TableName::new("a_top"), "SELECT id FROM b_middle"),
		view(TableName::new("b_middle"), "SELECT id FROM c_bottom"),
		view(TableName::new("c_bottom"), "SELECT id FROM posts"),
		view(TableName::new("d_alone"), "SELECT id FROM posts"),
	]);
	let names = |views: Vec<&View>| -> Vec<String> {
		views.iter().map(|view| view.name.to_string()).collect()
	};
	assert_eq!(
		names(schema.views_in_order().unwrap()),
		["c_bottom", "b_middle", "a_top", "d_alone"]
	);
	assert_eq!(
		names(schema.views_reading(&TableName::new("c_bottom"))),
		["b_middle", "a_top"]
	);
	schema.validate().unwrap();

	let cycle = blog(Dialect::Postgres).with_views(vec![
		view(TableName::new("left_view"), "SELECT id FROM right_view"),
		view(TableName::new("right_view"), "SELECT id FROM left_view"),
	]);
	assert_eq!(
		cycle.views_in_order().unwrap_err(),
		[TableName::new("left_view"), TableName::new("right_view")]
	);
	insta::assert_snapshot!(cycle.validate().unwrap_err().to_string(), @r"
	the schema has 1 problem(s):
	  - left_view, right_view: these views read each other, so none can be created first; a view is taken to read every view its query names
	");
}

#[test]
fn view_indexes_are_checked_and_recreate_the_view() {
	let name = TableName::new("titles");
	let mut view = View::new(
		name.clone(),
		vec![Column::new("title", ColumnType::TEXT)],
		Generated::new("SELECT title FROM posts"),
	);
	view.materialized = true;
	let schema = blog(Dialect::Postgres).with_views(vec![view.clone()]);

	let mut indexed = view.clone();
	indexed.indexes = vec![Index::unique(&name, &["title"])];
	let target = blog(Dialect::Postgres).with_views(vec![indexed.clone()]);
	target.validate().unwrap();
	assert_eq!(Schema::from_json(&target.to_json()).unwrap(), target);
	let json: serde_json::Value = serde_json::from_str(&schema.to_json()).unwrap();
	assert!(json["views"][0].get("indexes").is_none(), "{json}");
	let plan = diff(&schema, &target, &mut RequireHints).unwrap();
	assert_eq!(
		plan.steps,
		[
			Step::DropView(view.clone()),
			Step::CreateView(indexed.clone())
		]
	);

	let mut unmaterialized = indexed.clone();
	unmaterialized.materialized = false;
	unmaterialized.indexes.push(Index::new(&name, &["missing"]));
	unmaterialized.indexes.push(Index {
		name: "Bad".into(),
		columns: Vec::new(),
		unique: false,
		predicate: Some(Predicate::IsNull("gone".into())),
	});
	// An index named like a table's.
	unmaterialized
		.indexes
		.push(Index::unique(&TableName::new("users"), &["handle"]));
	let invalid = blog(Dialect::Postgres).with_views(vec![unmaterialized]);
	insta::assert_snapshot!(invalid.validate().unwrap_err().to_string());
}

fn searched_posts(columns: &[(&str, SearchWeight)], language: &str) -> Search {
	Search::new(
		language,
		columns
			.iter()
			.map(|(name, weight)| SearchColumn::new(*name, *weight))
			.collect(),
	)
}

#[test]
fn search_indexes_are_validated() {
	let mut schema = blog(Dialect::Sqlite);
	let posts = table_mut(&mut schema, "posts");
	posts
		.columns
		.push(Column::new("lk_search", ColumnType::TEXT));
	posts
		.columns
		.push(Column::new("title_key", ColumnType::TEXT).generated(Generated::new("lower(title)")));
	posts.search = Some(searched_posts(
		&[
			("title", SearchWeight::A),
			("title", SearchWeight::B),
			("score", SearchWeight::C),
			("missing", SearchWeight::D),
			("title_key", SearchWeight::D),
		],
		"german",
	));
	let users = table_mut(&mut schema, "users");
	users.search = Some(searched_posts(&[], "Not A Language"));
	users
		.indexes
		.push(Index::new(&TableName::new("users"), &["lk_search"]));
	users.indexes[1].name = Search::index_name(&TableName::new("users"));
	let tags = table_mut(&mut schema, "tags");
	tags.primary_key = vec!["lk_row".into()];
	tags.columns.push(Column::new("lk_row", ColumnType::Int64));
	tags.search = Some(searched_posts(&[("name", SearchWeight::A)], "simple"));
	// A table whose name is a search index's companion.
	schema.tables.push(Table::new(
		TableName::new("tags_lk_search"),
		vec![Column::new("id", ColumnType::Int64)],
		vec!["id".into()],
	));

	insta::assert_snapshot!(schema.validate().unwrap_err().to_string());

	// Postgres has every configuration it was built with.
	let postgres = Schema::new(
		Dialect::Postgres,
		vec![{
			let mut posts = table_mut(&mut blog(Dialect::Postgres), "posts").clone();
			posts.search = Some(searched_posts(&[("title", SearchWeight::A)], "german"));
			posts
		}],
	);
	assert!(
		postgres
			.problems()
			.iter()
			.all(|problem| !problem.location.contains("search")),
		"{:?}",
		postgres.problems()
	);
}

#[test]
fn search_indexes_round_trip_through_snapshots() {
	let schema = support::with_search(Dialect::Postgres);
	let json = schema.to_json();
	assert!(json.contains("\"weight\": \"A\""), "{json}");
	assert_eq!(Schema::from_json(&json).unwrap(), schema);

	// Snapshots written before search existed still read, and don't
	// mention it.
	let older = blog(Dialect::Postgres).to_json();
	assert!(!older.contains("search"));
}

#[test]
fn search_indexes_are_recreated_when_the_table_changes_under_them() {
	let before = support::with_search(Dialect::Postgres);
	let recreated = |after: &Schema| {
		let plan = diff(&before, after, &mut RequireHints).unwrap();
		let dropped = plan
			.steps
			.iter()
			.any(|step| matches!(step, Step::DropSearch { .. }));
		let created = plan
			.steps
			.iter()
			.any(|step| matches!(step, Step::CreateSearch { .. }));
		assert_eq!(dropped, created, "{plan:?}");
		assert!(plan.steps.iter().all(|step| {
			!matches!(step, Step::DropSearch { .. } | Step::CreateSearch { .. })
				|| step.risk() == Risk::Safe
		}));
		created
	};

	assert!(!recreated(&before));

	// A column it doesn't read changing type leaves it alone.
	let mut unread = before.clone();
	column_mut(table_mut(&mut unread, "posts"), "score").ty = ColumnType::Int64;
	assert!(!recreated(&unread));

	let mut retyped = before.clone();
	column_mut(table_mut(&mut retyped, "posts"), "body").ty = ColumnType::Text { max_len: Some(9) };
	assert!(recreated(&retyped));

	let mut rekeyed = before.clone();
	column_mut(table_mut(&mut rekeyed, "posts"), "id").ty = ColumnType::Int32;
	column_mut(table_mut(&mut rekeyed, "tags"), "post_id").ty = ColumnType::Int32;
	assert!(recreated(&rekeyed));

	let mut reweighted = before.clone();
	table_mut(&mut reweighted, "posts").search = Some(searched_posts(
		&[("title", SearchWeight::B), ("body", SearchWeight::B)],
		"simple",
	));
	assert!(recreated(&reweighted));

	let mut renamed = before.clone();
	let posts = table_mut(&mut renamed, "posts");
	posts.name = TableName::new("articles");
	posts.renamed_from = Some(TableName::new("posts"));
	posts.indexes[0].name = constraint_name(&posts.name, &["author_id"], "idx");
	posts.foreign_keys[0].name = constraint_name(&posts.name, &["author_id"], "fkey");
	table_mut(&mut renamed, "tags").foreign_keys[0].references = TableName::new("articles");
	assert!(recreated(&renamed));

	// Dropping the table drops its index, and adding one creates it.
	let mut dropped = before.clone();
	dropped.tables.retain(|table| table.name.name != "tags");
	let plan = diff(&dropped, &blog(Dialect::Postgres), &mut RequireHints).unwrap();
	assert!(
		plan.steps
			.iter()
			.any(|step| matches!(step, Step::DropSearch { .. }))
	);
	let empty = Schema::empty(Dialect::Postgres);
	let plan = diff(&empty, &before, &mut RequireHints).unwrap();
	assert_eq!(
		plan.steps
			.iter()
			.filter(|step| matches!(step, Step::CreateSearch { .. }))
			.count(),
		1
	);
	let without_posts = {
		let mut schema = before.clone();
		schema.tables.retain(|table| table.name.name == "users");
		schema
	};
	let plan = diff(&before, &without_posts, &mut RequireHints).unwrap();
	let rendered = render(&plan, &before, &without_posts);
	// On Postgres the column goes with its table.
	assert!(
		!rendered.sql().contains("DROP COLUMN"),
		"{}",
		rendered.sql()
	);
}

#[test]
fn row_security_is_validated_and_kept_in_snapshots() {
	let schema = support::with_row_security(Dialect::Postgres);
	assert!(schema.validate().is_ok());
	let json = schema.to_json();
	assert!(json.contains("\"command\": \"select\""), "{json}");
	assert_eq!(Schema::from_json(&json).unwrap(), schema);
	assert!(!blog(Dialect::Postgres).to_json().contains("row_security"));

	let mut broken = schema.clone();
	let security = table_mut(&mut broken, "posts")
		.row_security
		.as_mut()
		.unwrap();
	security.policies[1].using = Some("TRUE".into());
	security.policies[2].check = None;
	let duplicate = security.policies[0].clone();
	security.policies.push(duplicate);
	let mut on_sqlite = support::with_row_security(Dialect::Sqlite);
	on_sqlite.tables.retain(|table| table.name.name != "users");
	on_sqlite.tables.retain(|table| table.name.name != "tags");
	table_mut(&mut on_sqlite, "posts").foreign_keys.clear();

	insta::assert_snapshot!(format!(
		"{}\n{}",
		broken.validate().unwrap_err(),
		on_sqlite.validate().unwrap_err()
	));

	// Changing only the policies recreates them, and no view.
	let mut changed = support::with_views(Dialect::Postgres);
	table_mut(&mut changed, "posts").row_security = schema
		.tables
		.iter()
		.find(|table| table.name.name == "posts")
		.unwrap()
		.row_security
		.clone();
	let plan = diff(
		&support::with_views(Dialect::Postgres),
		&changed,
		&mut RequireHints,
	)
	.unwrap();
	assert!(
		plan.steps
			.iter()
			.all(|step| matches!(step, Step::CreateRowSecurity { .. })),
		"{plan:?}"
	);
	assert_eq!(
		plan.steps[0].to_string(),
		"+ row-level security on posts (SELECT, INSERT, UPDATE, DELETE)"
	);
	assert_eq!(plan.steps[0].risk(), Risk::Safe);

	// Renaming the table drops its policies under the old name.
	let mut renamed = schema.clone();
	let posts = table_mut(&mut renamed, "posts");
	posts.name = TableName::new("articles");
	posts.renamed_from = Some(TableName::new("posts"));
	posts.indexes[0].name = constraint_name(&posts.name, &["author_id"], "idx");
	posts.foreign_keys[0].name = constraint_name(&posts.name, &["author_id"], "fkey");
	table_mut(&mut renamed, "tags").foreign_keys[0].references = TableName::new("articles");
	let plan = diff(&schema, &renamed, &mut RequireHints).unwrap();
	let sql = render(&plan, &schema, &renamed).sql();
	assert!(
		sql.contains("DROP POLICY \"posts_select\" ON \"posts\""),
		"{sql}"
	);
	assert!(
		sql.contains("CREATE POLICY \"articles_select\" ON \"articles\""),
		"{sql}"
	);
	assert!(!sql.contains("DISABLE ROW LEVEL SECURITY"), "{sql}");
}
