//! Schema pairs that every test suite runs: the render snapshots, and the
//! apply tests against real SQLite and Postgres databases.

#![allow(dead_code, reason = "each test binary uses a different part")]

use leptoskit_db_schema::*;

/// A migration from `before` to `after`, with rows to insert first.
pub struct Scenario {
	pub name: &'static str,
	pub before: fn(Dialect) -> Schema,
	pub after: fn(Dialect) -> Schema,
	/// Whether the diff may treat ambiguous changes as new columns.
	pub no_renames: bool,
	/// The one dialect the scenario applies to, when it uses features the
	/// other lacks.
	pub only: Option<Dialect>,
	/// Statements that insert rows into `before`, valid in both dialects.
	pub seed: &'static [&'static str],
	/// Each surviving table, by its new flat name, with the rows it should
	/// still have.
	pub expected_rows: &'static [(&'static str, i64)],
}

impl Scenario {
	pub fn plan(&self, dialect: Dialect) -> Plan {
		let before = (self.before)(dialect);
		let after = (self.after)(dialect);

		if self.no_renames {
			diff(&before, &after, &mut NoRenames).expect("the scenario diffs")
		} else {
			diff(&before, &after, &mut RequireHints).expect("the scenario diffs")
		}
	}

	pub fn migration(&self, dialect: Dialect) -> Migration {
		render(
			&self.plan(dialect),
			&(self.before)(dialect),
			&(self.after)(dialect),
		)
	}
}

/// The SQL that creates `schema` from nothing.
pub fn create(schema: &Schema) -> Migration {
	let schema = schema.without_rename_hints();
	let empty = Schema::empty(schema.dialect);
	let plan = diff(&empty, &schema, &mut RequireHints).expect("a fresh schema diffs");
	render(&plan, &empty, &schema)
}

#[expect(
	clippy::too_many_lines,
	reason = "one entry per scenario reads best as a single table"
)]
pub fn scenarios() -> Vec<Scenario> {
	vec![
		Scenario {
			name: "create_tables",
			before: Schema::empty,
			after: blog,
			no_renames: false,
			only: None,
			seed: &[],
			expected_rows: &[],
		},
		Scenario {
			name: "add_columns",
			before: blog,
			after: |dialect| {
				let mut schema = blog(dialect);
				let users = table_mut(&mut schema, "users");
				users
					.columns
					.push(Column::new("bio", ColumnType::TEXT).nullable());
				users.columns.push(
					Column::new("karma", ColumnType::Int64).default(DefaultValue::Integer(0)),
				);
				users.columns.push(
					Column::new("joined_at", ColumnType::Timestamp).default(DefaultValue::Now),
				);
				users.columns.push(
					Column::new("motto", ColumnType::TEXT)
						.default(DefaultValue::Sql("'n/a'".into())),
				);
				schema
			},
			no_renames: false,
			only: None,
			seed: SEED,
			expected_rows: &[("users", 2), ("posts", 3)],
		},
		Scenario {
			name: "rename_table_and_column",
			before: blog,
			after: |dialect| {
				let mut schema = blog(dialect);
				let users = table_mut(&mut schema, "users");
				users.name = TableName::new("members");
				users.renamed_from = Some(TableName::new("users"));
				users.indexes = vec![Index::unique(&TableName::new("members"), &["username"])];
				let handle = users
					.columns
					.iter_mut()
					.find(|column| column.name == "handle")
					.unwrap();
				handle.name = "username".into();
				handle.renamed_from = Some("handle".into());

				let posts = table_mut(&mut schema, "posts");
				posts.foreign_keys[0].references = TableName::new("members");
				Schema::new(dialect, schema.tables)
			},
			no_renames: false,
			only: None,
			seed: SEED,
			expected_rows: &[("members", 2), ("posts", 3)],
		},
		Scenario {
			name: "alter_columns",
			before: blog,
			after: |dialect| {
				let mut schema = blog(dialect);
				let users = table_mut(&mut schema, "users");
				column_mut(users, "handle").ty = ColumnType::Text { max_len: Some(64) };

				let posts = table_mut(&mut schema, "posts");
				column_mut(posts, "score").ty = ColumnType::Int64;
				let body = column_mut(posts, "body");
				body.nullable = false;
				body.default = Some(DefaultValue::Text(String::new()));
				schema
			},
			no_renames: false,
			only: None,
			seed: SEED,
			expected_rows: &[("users", 2), ("posts", 3)],
		},
		Scenario {
			name: "constraints",
			before: blog,
			after: |dialect| {
				let mut schema = blog(dialect);
				let posts_name = TableName::new("posts");
				let posts = table_mut(&mut schema, "posts");
				posts
					.checks
					.retain(|check| check.name != "posts_status_check");
				posts.checks.push(Check {
					name: "posts_title_check".into(),
					rule: CheckRule::Length {
						column: "title".into(),
						min: Some(1),
						max: None,
					},
				});
				posts.indexes = vec![Index::unique(&posts_name, &["author_id", "title"])];
				posts
					.columns
					.push(Column::new("editor_id", ColumnType::Int64).nullable());
				posts.foreign_keys.push(
					ForeignKey::new(
						&posts_name,
						&["editor_id"],
						TableName::new("users"),
						&["id"],
					)
					.on_delete(ReferentialAction::SetNull),
				);
				schema
			},
			no_renames: false,
			only: None,
			seed: SEED,
			expected_rows: &[("users", 2), ("posts", 3)],
		},
		Scenario {
			name: "primary_key_change",
			before: blog,
			after: |dialect| {
				let mut schema = blog(dialect);
				let tags = table_mut(&mut schema, "tags");
				tags.primary_key = vec!["post_id".into(), "name".into()];
				schema
			},
			no_renames: false,
			only: None,
			seed: SEED,
			expected_rows: &[("tags", 2)],
		},
		Scenario {
			name: "drop_column_and_tables",
			before: blog,
			after: |dialect| {
				let mut schema = blog(dialect);
				schema.tables.retain(|table| table.name.name == "users");
				let users = table_mut(&mut schema, "users");
				users.columns.retain(|column| column.name != "display_name");
				schema
			},
			no_renames: false,
			only: None,
			seed: SEED,
			expected_rows: &[("users", 2)],
		},
		Scenario {
			name: "replace_column_without_renames",
			before: blog,
			after: |dialect| {
				let mut schema = blog(dialect);
				let users = table_mut(&mut schema, "users");
				let display_name = column_mut(users, "display_name");
				display_name.name = "full_name".into();
				display_name.nullable = true;
				schema
			},
			no_renames: true,
			only: None,
			seed: SEED,
			expected_rows: &[("users", 2)],
		},
		Scenario {
			name: "retype_referenced_key",
			before: blog,
			after: |dialect| {
				let mut schema = blog(dialect);
				column_mut(table_mut(&mut schema, "users"), "id").ty = ColumnType::Int32;
				column_mut(table_mut(&mut schema, "posts"), "author_id").ty = ColumnType::Int32;
				schema
			},
			no_renames: false,
			only: None,
			seed: SEED,
			expected_rows: &[("users", 2), ("posts", 3), ("tags", 2)],
		},
		Scenario {
			name: "every_type_and_a_partial_unique",
			before: blog,
			after: |dialect| {
				let mut schema = blog(dialect);
				let events_name = TableName::new("events");
				let mut events = Table::new(
					events_name.clone(),
					vec![
						Column::new("id", ColumnType::Uuid),
						Column::new("slug", ColumnType::Text { max_len: Some(40) }),
						Column::new("public", ColumnType::Bool).default(DefaultValue::Bool(true)),
						Column::new("seats", ColumnType::Int16).default(DefaultValue::Integer(10)),
						Column::new("ratio", ColumnType::Float32).nullable(),
						Column::new("weight", ColumnType::Float64)
							.default(DefaultValue::Float(1.5)),
						Column::new(
							"price",
							ColumnType::Decimal {
								precision: Some(12),
								scale: Some(2),
							},
						)
						.nullable(),
						Column::new("cover", ColumnType::Bytes).nullable(),
						Column::new("details", ColumnType::Json).nullable(),
						Column::new("starts_on", ColumnType::Date),
						Column::new("created_at", ColumnType::Timestamp).default(DefaultValue::Now),
						Column::new("deleted_at", ColumnType::Timestamp).nullable(),
					],
					vec!["id".into()],
				);
				events.indexes.push(
					Index::unique(&events_name, &["slug"])
						.partial(Predicate::IsNull("deleted_at".into())),
				);
				schema.tables.push(events);
				Schema::new(dialect, schema.tables)
			},
			no_renames: false,
			only: None,
			seed: SEED,
			expected_rows: &[("users", 2), ("events", 0)],
		},
		Scenario {
			name: "namespaces_and_comments",
			before: blog,
			after: |dialect| {
				let mut schema = blog(dialect);
				let sessions_name = TableName::namespaced("auth", "sessions");
				let mut sessions = Table::new(
					sessions_name.clone(),
					vec![
						Column::new("id", ColumnType::Int64),
						Column::new("user_id", ColumnType::Int64),
						Column::new("expires_at", ColumnType::Timestamp),
					],
					vec!["id".into()],
				);
				sessions.comment = Some("Signed-in devices.".into());
				sessions.foreign_keys.push(
					ForeignKey::new(
						&sessions_name,
						&["user_id"],
						TableName::new("users"),
						&["id"],
					)
					.on_delete(ReferentialAction::Cascade),
				);
				sessions
					.indexes
					.push(Index::new(&sessions_name, &["user_id"]));
				schema.tables.push(sessions);

				let users = table_mut(&mut schema, "users");
				users.comment = Some("Someone with an account.".into());
				column_mut(users, "handle").comment = Some("Unique, shown on profiles.".into());
				Schema::new(dialect, schema.tables)
			},
			no_renames: false,
			only: None,
			seed: SEED,
			expected_rows: &[("users", 2), ("auth_sessions", 0)],
		},
		Scenario {
			name: "rename_child_table",
			before: blog,
			after: |dialect| {
				let mut schema = blog(dialect);
				let articles = TableName::new("articles");
				let posts = table_mut(&mut schema, "posts");
				posts.name = articles.clone();
				posts.renamed_from = Some(TableName::new("posts"));
				posts.indexes = vec![Index::new(&articles, &["author_id"])];
				posts.foreign_keys[0].name = constraint_name(&articles, &["author_id"], "fkey");

				for check in &mut posts.checks {
					check.name = check.name.replace("posts_", "articles_");
				}

				table_mut(&mut schema, "tags").foreign_keys[0].references = articles;
				Schema::new(dialect, schema.tables)
			},
			no_renames: false,
			only: None,
			seed: SEED,
			expected_rows: &[("articles", 3), ("tags", 2)],
		},
		Scenario {
			name: "retype_to_text_and_drop_checked_column",
			before: blog,
			after: |dialect| {
				let mut schema = blog(dialect);
				let posts = table_mut(&mut schema, "posts");
				posts
					.checks
					.retain(|check| check.name != "posts_score_check");
				let score = column_mut(posts, "score");
				score.ty = ColumnType::TEXT;
				score.default = Some(DefaultValue::Text("0".into()));
				posts.columns.retain(|column| column.name != "status");
				posts
					.checks
					.retain(|check| check.name != "posts_status_check");
				schema
			},
			no_renames: false,
			only: None,
			seed: SEED,
			expected_rows: &[("posts", 3)],
		},
		Scenario {
			name: "drop_tables_in_a_cycle",
			before: |dialect| {
				let mut schema = blog(dialect);
				schema.tables.extend(cycle());
				Schema::new(dialect, schema.tables)
			},
			after: blog,
			no_renames: false,
			only: None,
			seed: CYCLE_SEED,
			expected_rows: &[("users", 2)],
		},
		Scenario {
			name: "postgres_types_namespaces_and_constraints",
			before: |dialect| {
				let mut schema = blog(dialect);
				table_mut(&mut schema, "users").comment = Some("Someone with an account.".into());
				schema
			},
			after: |dialect| {
				let mut schema = blog(dialect);
				let content_tags = TableName::namespaced("content", "tags");
				let tags = table_mut(&mut schema, "tags");
				tags.name = content_tags.clone();
				tags.renamed_from = Some(TableName::new("tags"));
				tags.foreign_keys[0].name = constraint_name(&content_tags, &["post_id"], "fkey");
				tags.columns.push(
					Column::new(
						"aliases",
						ColumnType::Array {
							element: Box::new(ColumnType::TEXT),
						},
					)
					.nullable(),
				);
				tags.indexes.push(
					Index::new(&content_tags, &["name"])
						.partial(Predicate::IsNotNull("aliases".into())),
				);

				let posts_name = TableName::new("posts");
				let posts = table_mut(&mut schema, "posts");
				posts.columns.push(
					Column::new(
						"price",
						ColumnType::Decimal {
							precision: Some(10),
							scale: None,
						},
					)
					.nullable(),
				);
				posts.columns.push(
					Column::new(
						"cost",
						ColumnType::Decimal {
							precision: None,
							scale: None,
						},
					)
					.nullable(),
				);
				posts.columns.push(
					Column::new("slug", ColumnType::TEXT)
						.default(DefaultValue::Sql("lower('untitled')".into())),
				);
				posts.columns.push(
					Column::new("ratio", ColumnType::Float64).default(DefaultValue::Float(2.0)),
				);
				posts.checks.push(Check {
					name: "posts_slug_check".into(),
					rule: CheckRule::Regex {
						column: "slug".into(),
						pattern: "^[a-z0-9-]+$".into(),
					},
				});
				posts.checks.push(Check {
					name: "posts_ratio_check".into(),
					rule: CheckRule::Sql {
						sql: "ratio > 0".into(),
					},
				});
				posts.indexes.push(
					Index::new(&posts_name, &["title"])
						.partial(Predicate::Sql("body IS NOT NULL".into())),
				);
				posts.foreign_keys[0].deferred = true;
				posts.foreign_keys[0].on_delete = ReferentialAction::Restrict;
				Schema::new(dialect, schema.tables)
			},
			no_renames: false,
			only: Some(Dialect::Postgres),
			seed: SEED,
			expected_rows: &[("content_tags", 2), ("posts", 3)],
		},
	]
}

/// Rows for [`blog`], valid in both dialects.
const SEED: &[&str] = &[
	"INSERT INTO users (id, handle, display_name, created_at) VALUES (1, 'ada', 'Ada', \
	 '2026-01-01T00:00:00.000Z')",
	"INSERT INTO users (id, handle, display_name, created_at) VALUES (2, 'grace', 'Grace', \
	 '2026-01-02T00:00:00.000Z')",
	"INSERT INTO posts (id, author_id, title, body, score, status) VALUES (10, 1, 'Hello', \
	 'First', 5, 'draft')",
	"INSERT INTO posts (id, author_id, title, body, score, status) VALUES (11, 1, 'Again', NULL, \
	 50, 'published')",
	"INSERT INTO posts (id, author_id, title, body, score, status) VALUES (12, 2, 'Mine', 'Hi', \
	 100, 'published')",
	"INSERT INTO tags (post_id, name) VALUES (10, 'intro')",
	"INSERT INTO tags (post_id, name) VALUES (11, 'intro')",
];

/// Users, their posts, and tags on posts.
pub fn blog(dialect: Dialect) -> Schema {
	let users_name = TableName::new("users");
	let mut users = Table::new(
		users_name.clone(),
		vec![
			Column::new("id", ColumnType::Int64),
			Column::new("handle", ColumnType::Text { max_len: Some(32) }),
			Column::new("display_name", ColumnType::TEXT),
			Column::new("created_at", ColumnType::Timestamp),
		],
		vec!["id".into()],
	);
	users.indexes.push(Index::unique(&users_name, &["handle"]));

	let posts_name = TableName::new("posts");
	let mut posts = Table::new(
		posts_name.clone(),
		vec![
			Column::new("id", ColumnType::Int64),
			Column::new("author_id", ColumnType::Int64),
			Column::new("title", ColumnType::TEXT),
			Column::new("body", ColumnType::TEXT).nullable(),
			Column::new("score", ColumnType::Int32).default(DefaultValue::Integer(0)),
			Column::new("status", ColumnType::TEXT).default(DefaultValue::Text("draft".into())),
		],
		vec!["id".into()],
	);
	posts.indexes.push(Index::new(&posts_name, &["author_id"]));
	posts.foreign_keys.push(
		ForeignKey::new(&posts_name, &["author_id"], users_name.clone(), &["id"])
			.on_delete(ReferentialAction::Cascade),
	);
	posts.checks.push(Check {
		name: "posts_score_check".into(),
		rule: CheckRule::Range {
			column: "score".into(),
			min: Some(0.0),
			max: Some(100.0),
		},
	});
	posts.checks.push(Check {
		name: "posts_status_check".into(),
		rule: CheckRule::OneOf {
			column: "status".into(),
			values: vec!["draft".into(), "published".into()],
		},
	});

	// Keyed by post alone, so that a scenario can widen the key.
	let tags_name = TableName::new("tags");
	let mut tags = Table::new(
		tags_name.clone(),
		vec![
			Column::new("id", ColumnType::Int64).default(DefaultValue::Integer(0)),
			Column::new("post_id", ColumnType::Int64),
			Column::new("name", ColumnType::TEXT),
		],
		vec!["post_id".into()],
	);
	tags.foreign_keys.push(
		ForeignKey::new(&tags_name, &["post_id"], posts_name, &["id"])
			.on_delete(ReferentialAction::Cascade),
	);

	Schema::new(dialect, vec![users, posts, tags])
}

pub fn table_mut<'a>(schema: &'a mut Schema, name: &str) -> &'a mut Table {
	schema
		.tables
		.iter_mut()
		.find(|table| table.name.name == name)
		.expect("the table exists")
}

pub fn column_mut<'a>(table: &'a mut Table, name: &str) -> &'a mut Column {
	table
		.columns
		.iter_mut()
		.find(|column| column.name == name)
		.expect("the column exists")
}

/// Two tables that reference each other.
fn cycle() -> Vec<Table> {
	let left = TableName::new("left_side");
	let right = TableName::new("right_side");
	let table = |name: &TableName, other: &TableName| {
		let mut table = Table::new(
			name.clone(),
			vec![
				Column::new("id", ColumnType::Int64),
				Column::new("other_id", ColumnType::Int64).nullable(),
			],
			vec!["id".into()],
		);
		table
			.foreign_keys
			.push(ForeignKey::new(name, &["other_id"], other.clone(), &["id"]));
		table
	};

	vec![table(&left, &right), table(&right, &left)]
}

/// Rows for [`blog`] plus [`cycle`], so that dropping the cycle has rows to
/// trip over.
const CYCLE_SEED: &[&str] = &[
	"INSERT INTO users (id, handle, display_name, created_at) VALUES (1, 'ada', 'Ada', \
	 '2026-01-01T00:00:00.000Z')",
	"INSERT INTO users (id, handle, display_name, created_at) VALUES (2, 'grace', 'Grace', \
	 '2026-01-02T00:00:00.000Z')",
	"INSERT INTO left_side (id, other_id) VALUES (1, NULL)",
	"INSERT INTO right_side (id, other_id) VALUES (1, 1)",
	"UPDATE left_side SET other_id = 1 WHERE id = 1",
];
