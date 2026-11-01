//! Parses `#[db::table(...)]` and the `#[db(...)]` attributes on its fields.

use proc_macro2::Span;
use proc_macro2::TokenStream;
use syn::Attribute;
use syn::Expr;
use syn::Ident;
use syn::Lit;
use syn::LitStr;
use syn::Path;
use syn::Token;
use syn::meta::ParseNestedMeta;
use syn::parenthesized;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;

/// The options on `#[db::table(...)]`.
pub(crate) struct TableArgs {
	pub name: LitStr,
	pub namespace: Option<LitStr>,
	/// Whether clients get a public type, and its name.
	pub public: Option<PublicName>,
	pub indexes: Vec<Vec<Ident>>,
	pub uniques: Vec<Vec<Ident>>,
	pub checks: Vec<(LitStr, LitStr)>,
	pub renamed_from: Option<LitStr>,
	/// The schema pack that owns the table: only its crate writes to it.
	pub pack: Option<LitStr>,
	/// How its rows move when accounts merge, as the `MergeStrategy` variant
	/// `on_account_merge = keep_newest` names.
	pub on_account_merge: Option<Ident>,
	pub krate: Option<Path>,
	/// The client types besides `public` that see some `private` columns.
	pub projections: Vec<Ident>,
	/// What a `#[db::link]` table links.
	pub link: Option<LinkArgs>,
	/// The hooks the table implements: `hooks(before_write, after_update)`.
	pub hooks: Vec<Ident>,
	/// Every change is recorded in the audit log.
	pub audit: bool,
	/// Every write to the table is captured in the change log, by triggers
	/// that migrations create, for live subscribers.
	pub live: bool,
	/// Declared by the database layer itself, such as the audit log: not
	/// registered for export, and written only from inside its crate.
	pub internal: bool,
	/// What a `#[db::view]` selects.
	pub view: Option<ViewArgs>,
	/// The language searched columns' words are in: `search(language =
	/// "english")`.
	pub search_language: Option<LitStr>,
	/// The polymorphic enum the table is a variant of: `extends = Actor`.
	pub extends: Option<Path>,
}

/// The query behind a `#[db::view]`.
pub(crate) struct ViewArgs {
	/// The query on Postgres and on SQLite.
	pub sql: (LitStr, LitStr),
	/// It keeps its rows until refreshed.
	pub materialized: bool,
	/// The tables its query reads, `reads(Post, User)`, which a tracked
	/// read records in its place.
	pub reads: Option<Vec<Path>>,
}

/// The first argument `#[db::view]` passes `#[db::table]`.
pub(crate) const VIEW_MARKER: &str = "__view";

/// The hooks a table can declare.
pub(crate) const HOOKS: [&str; 4] = [
	"before_write",
	"after_insert",
	"after_update",
	"after_delete",
];

/// The hidden argument the database layer's own tables pass.
pub(crate) const INTERNAL_MARKER: &str = "__internal";

/// The ends of a `#[db::link]` table.
pub(crate) struct LinkArgs {
	pub from: LinkEnd,
	pub to: LinkEnd,
	/// Each row of `to` may be linked from one row of `from` at most.
	pub exclusive: bool,
}

/// One end of a link: the table, the method it gets, and the column that
/// holds its key.
pub(crate) struct LinkEnd {
	pub table: Path,
	pub method: Option<Ident>,
	pub column: Option<LitStr>,
}

impl LinkEnd {
	/// `Post` or `Post as tags`.
	fn parse(meta: &ParseNestedMeta<'_>) -> syn::Result<Self> {
		let value = meta.value()?;
		let table: Path = value.parse()?;
		let method = if value.peek(Token![as]) {
			value.parse::<Token![as]>()?;
			Some(value.parse()?)
		} else {
			None
		};

		Ok(Self {
			table,
			method,
			column: None,
		})
	}
}

/// The first argument `#[db::link]` passes `#[db::table]`, so that the
/// table's expansion knows it is a link's, even after a mixin's macro hands
/// it back.
pub(crate) const LINK_MARKER: &str = "__link";

/// The strategies `on_account_merge` takes.
const MERGE_STRATEGIES: &[(&str, &str)] = &[
	("reassign", "Reassign"),
	("reassign_skip_conflicts", "ReassignSkipConflicts"),
	("keep_survivor", "KeepSurvivor"),
	("keep_newest", "KeepNewest"),
	("custom", "Custom"),
];

/// `on_account_merge = keep_newest`, as the `MergeStrategy` variant it names.
fn merge_strategy(meta: &ParseNestedMeta<'_>) -> syn::Result<Ident> {
	let strategy: Ident = meta.value()?.parse()?;
	let (_, variant) = MERGE_STRATEGIES
		.iter()
		.find(|(name, _)| strategy == name)
		.ok_or_else(|| {
			syn::Error::new(
				strategy.span(),
				"`on_account_merge` is `reassign`, `reassign_skip_conflicts`, `keep_survivor`, \
				 `keep_newest`, or `custom`",
			)
		})?;
	Ok(Ident::new(variant, strategy.span()))
}

/// Refuses a link's own options on a plain table, and `unique` on a link,
/// whose two ends are already its key.
fn check_link_option(meta: &ParseNestedMeta<'_>, is_link: bool) -> syn::Result<()> {
	let link_option = ["from", "to", "from_column", "to_column", "exclusive"]
		.iter()
		.any(|option| meta.path.is_ident(option));

	if link_option && !is_link {
		return Err(meta
			.error("`from`, `to`, and `exclusive` describe a link table: use `#[db::link(...)]`"));
	}

	if is_link && meta.path.is_ident("unique") && !meta.input.peek(syn::token::Paren) {
		return Err(meta.error(
			"a link already holds each pair of rows at most once: its two ends are its key, so \
			 `unique` isn't needed. `exclusive` lets each `to` row be linked from one `from` row \
			 only",
		));
	}

	Ok(())
}

impl TableArgs {
	pub fn parse(tokens: TokenStream) -> syn::Result<Self> {
		let mut name = None;
		let mut namespace = None;
		let mut public = None;
		let mut indexes = Vec::new();
		let mut uniques = Vec::new();
		let mut checks = Vec::new();
		let mut renamed_from = None;
		let mut pack = None;

		let mut on_account_merge = None;
		let mut krate = None;
		let mut projections = Vec::new();
		let mut is_link = false;
		let mut from = None;
		let mut to = None;
		let mut from_column = None;
		let mut to_column = None;
		let mut exclusive = false;
		let mut hooks = Vec::new();
		let mut audit = false;
		let mut live = false;
		let mut internal = false;
		let mut is_view = false;
		let mut view = ViewOptions::default();
		let mut search_language = None;
		let mut extends = None;

		let parser = syn::meta::parser(|meta| {
			if meta.path.is_ident(VIEW_MARKER) {
				is_view = true;
				return Ok(());
			}

			if view_option(&meta, is_view, &mut view)? {
				return Ok(());
			}

			check_link_option(&meta, is_link)?;

			if meta.path.is_ident(LINK_MARKER) {
				is_link = true;
			} else if meta.path.is_ident(INTERNAL_MARKER) {
				internal = true;
			} else if meta.path.is_ident("audit") {
				audit = true;
			} else if meta.path.is_ident("live") {
				live = true;
			} else if meta.path.is_ident("hooks") {
				hooks = hook_list(&meta)?;
			} else if meta.path.is_ident("from") {
				from = Some(LinkEnd::parse(&meta)?);
			} else if meta.path.is_ident("to") {
				to = Some(LinkEnd::parse(&meta)?);
			} else if meta.path.is_ident("from_column") {
				from_column = Some(string(&meta)?);
			} else if meta.path.is_ident("to_column") {
				to_column = Some(string(&meta)?);
			} else if meta.path.is_ident("exclusive") {
				exclusive = true;
			} else if meta.path.is_ident("projection") {
				projections.extend(idents(&meta)?);
			} else if meta.path.is_ident("name") {
				name = Some(string(&meta)?);
			} else if meta.path.is_ident("namespace") {
				namespace = Some(string(&meta)?);
			} else if meta.path.is_ident("public") {
				public = Some(if meta.input.peek(Token![=]) {
					PublicName::Named(meta.value()?.parse()?)
				} else {
					PublicName::Default
				});
			} else if meta.path.is_ident("index") {
				indexes.push(idents(&meta)?);
			} else if meta.path.is_ident("unique") {
				uniques.push(idents(&meta)?);
			} else if meta.path.is_ident("check") {
				checks.push(check(&meta)?);
			} else if meta.path.is_ident("renamed_from") {
				renamed_from = Some(string(&meta)?);
			} else if meta.path.is_ident("pack") {
				pack = Some(string(&meta)?);
			} else if meta.path.is_ident("on_account_merge") {
				on_account_merge = Some(merge_strategy(&meta)?);
			} else if meta.path.is_ident("crate") {
				krate = Some(string(&meta)?.parse()?);
			} else if meta.path.is_ident("search") {
				search_language = Some(search_language_option(&meta)?);
			} else if meta.path.is_ident("extends") && !is_link && !is_view {
				extends = Some(meta.value()?.parse()?);
			} else {
				return Err(meta.error(unknown_table_option(is_link)));
			}

			Ok(())
		});
		syn::parse::Parser::parse2(parser, tokens)?;

		let name = name.ok_or_else(|| {
			syn::Error::new(
				Span::call_site(),
				"a table needs an explicit name, such as #[db::table(name = \"users\")]: renaming \
				 the struct then never renames the table",
			)
		})?;

		let keeps_indexes = !indexes.is_empty() || !uniques.is_empty();
		let view = is_view
			.then(|| ViewArgs::new(view, keeps_indexes))
			.transpose()?;

		if live && view.is_some() {
			return Err(syn::Error::new(
				Span::call_site(),
				"a view has no writes to capture: `live` is for tables",
			));
		}

		let link = if is_link {
			let (Some(mut from), Some(mut to)) = (from, to) else {
				return Err(syn::Error::new(
					Span::call_site(),
					"a link names both its ends: `from = Post as tags, to = Tag as posts`",
				));
			};
			from.column = from_column;
			to.column = to_column;
			Some(LinkArgs {
				from,
				to,
				exclusive,
			})
		} else {
			None
		};

		Ok(Self {
			name,
			// A pack's tables live in its namespace unless they name another.
			namespace: namespace.or_else(|| pack.clone()),
			projections,
			link,
			public,
			indexes,
			uniques,
			checks,
			renamed_from,
			pack,
			on_account_merge,
			krate,
			hooks,
			audit,
			live,
			internal,
			view,
			search_language,
			extends,
		})
	}

	/// Whether the table declares `hook`.
	pub fn has_hook(&self, hook: &str) -> bool {
		self.hooks.iter().any(|declared| declared == hook)
	}
}

/// What an unknown option on `#[db::table]` or `#[db::link]` says.
fn unknown_table_option(is_link: bool) -> &'static str {
	if is_link {
		"unknown link option; expected `name`, `from`, `to`, `from_column`, `to_column`, \
		 `exclusive`, `namespace`, `public`, `projection`, `index`, `check`, `hooks`, `audit`, \
		 `live`, `search`, `renamed_from`, `pack`, or `crate`"
	} else {
		"unknown table option; expected `name`, `namespace`, `public`, `projection`, `index`, \
		 `unique`, `check`, `hooks`, `audit`, `live`, `search`, `extends`, `renamed_from`, `pack`, \
		 `on_account_merge`, or `crate`"
	}
}

/// `search(language = "english")`.
fn search_language_option(meta: &ParseNestedMeta<'_>) -> syn::Result<LitStr> {
	const HINT: &str = "`search` takes `language = \"…\"`, such as `search(language = \
	                    \"english\")`; mark the searched columns with `#[db(search)]`";

	if !meta.input.peek(syn::token::Paren) {
		return Err(meta.error(HINT));
	}

	let mut language = None;
	meta.parse_nested_meta(|inner| {
		if inner.path.is_ident("language") {
			language = Some(string(&inner)?);
			Ok(())
		} else {
			Err(inner.error(HINT))
		}
	})?;
	language.ok_or_else(|| meta.error(HINT))
}

/// `search` or `search(weight = A)`, the weight one of `A` to `D`.
fn search_weight(meta: &ParseNestedMeta<'_>) -> syn::Result<Ident> {
	const HINT: &str = "`search` takes `weight = A`, `B`, `C`, or `D`: how much a match in this \
	                    column counts, `A` most";

	if !meta.input.peek(syn::token::Paren) {
		return Ok(Ident::new("D", meta.path.span()));
	}

	let mut weight = None;
	meta.parse_nested_meta(|inner| {
		if !inner.path.is_ident("weight") {
			return Err(inner.error(HINT));
		}

		let ident: Ident = inner.value()?.parse()?;

		if !["A", "B", "C", "D"].contains(&ident.to_string().as_str()) {
			return Err(syn::Error::new(ident.span(), HINT));
		}

		weight = Some(ident);
		Ok(())
	})?;
	weight.ok_or_else(|| meta.error(HINT))
}

/// `check(name = "…", sql = "…")`.
fn check(meta: &ParseNestedMeta<'_>) -> syn::Result<(LitStr, LitStr)> {
	let mut check_name = None;
	let mut sql = None;
	meta.parse_nested_meta(|inner| {
		if inner.path.is_ident("name") {
			check_name = Some(string(&inner)?);
		} else if inner.path.is_ident("sql") {
			sql = Some(string(&inner)?);
		} else {
			return Err(inner.error("`check` takes `name = \"...\"` and `sql = \"...\"`"));
		}

		Ok(())
	})?;

	match (check_name, sql) {
		(Some(check_name), Some(sql)) => Ok((check_name, sql)),
		_ => Err(meta.error("`check` needs both `name = \"...\"` and `sql = \"...\"`")),
	}
}

/// `hooks(before_write, …)`, each a known hook, once.
fn hook_list(meta: &ParseNestedMeta<'_>) -> syn::Result<Vec<Ident>> {
	let mut hooks: Vec<Ident> = Vec::new();

	for hook in idents(meta)? {
		if !HOOKS.contains(&hook.to_string().as_str()) {
			return Err(syn::Error::new(
				hook.span(),
				"a table's hooks are `before_write`, `after_insert`, `after_update`, and \
				 `after_delete`",
			));
		}

		if hooks.contains(&hook) {
			return Err(syn::Error::new(
				hook.span(),
				format!("`{hook}` is listed twice"),
			));
		}

		hooks.push(hook);
	}

	Ok(hooks)
}

/// Reads a view's own options, `sql` and `materialized`, and refuses the
/// table options a view can't take, or a view's options on a table.
/// Whether `meta` was a view's own option.
fn view_option(
	meta: &ParseNestedMeta<'_>,
	is_view: bool,
	view: &mut ViewOptions,
) -> syn::Result<bool> {
	const VIEW_OPTIONS: &[&str] = &[
		"name",
		"namespace",
		"public",
		"projection",
		"index",
		"unique",
		"crate",
	];
	let own = meta.path.is_ident("sql")
		|| meta.path.is_ident("materialized")
		|| meta.path.is_ident("reads");

	if !is_view {
		return if own {
			Err(meta.error(
				"`sql`, `materialized`, and `reads` describe a view: use `#[db::view(...)]`",
			))
		} else {
			Ok(false)
		};
	}

	if meta.path.is_ident("sql") {
		view.sql = Some(dialect_sql(meta, "sql")?);
	} else if meta.path.is_ident("materialized") {
		view.materialized = true;
	} else if meta.path.is_ident("reads") {
		if view.reads.is_some() {
			return Err(meta.error("`reads` is given twice; list every table in one"));
		}

		let content;
		parenthesized!(content in meta.input);
		let tables = content.parse_terminated(Path::parse_mod_style, Token![,])?;

		if tables.is_empty() {
			return Err(meta.error("name the tables the view's query reads: `reads(Post, User)`"));
		}

		view.reads = Some(tables.into_iter().collect());
	} else if !VIEW_OPTIONS.iter().any(|option| meta.path.is_ident(option)) {
		return Err(meta.error(
			"unknown view option; expected `name`, `sql`, `materialized`, `reads`, `namespace`, \
			 `public`, `projection`, `index`, `unique`, or `crate`",
		));
	}

	Ok(own)
}

/// A view's own options, as `#[db::view(...)]` gives them.
#[derive(Default)]
struct ViewOptions {
	sql: Option<(LitStr, LitStr)>,
	materialized: bool,
	reads: Option<Vec<Path>>,
}

impl ViewArgs {
	/// A view's options, refusing indexes on one that keeps no rows.
	fn new(options: ViewOptions, indexed: bool) -> syn::Result<Self> {
		let ViewOptions {
			sql,
			materialized,
			reads,
		} = options;

		if indexed && !materialized {
			return Err(syn::Error::new(
				Span::call_site(),
				"only a materialized view keeps rows to index: add `materialized`, or drop \
				 `index(...)` and `unique(...)`",
			));
		}

		let sql = sql.ok_or_else(|| {
			syn::Error::new(
				Span::call_site(),
				"a view needs its query: `sql = \"SELECT …\"`, or `sql(postgres = \"…\", sqlite = \
				 \"…\")`",
			)
		})?;
		Ok(Self {
			sql,
			materialized,
			reads,
		})
	}
}

/// The options on `#[db::mixin(...)]`: indexes and unique constraints over
/// its own fields, which every table that flattens it inherits.
pub(crate) struct MixinArgs {
	pub indexes: Vec<Vec<Ident>>,
	pub uniques: Vec<Vec<Ident>>,
	pub krate: Option<Path>,
}

impl MixinArgs {
	pub fn parse(tokens: TokenStream) -> syn::Result<Self> {
		let mut indexes = Vec::new();
		let mut uniques = Vec::new();
		let mut krate = None;

		let parser = syn::meta::parser(|meta| {
			if meta.path.is_ident("index") {
				indexes.push(idents(&meta)?);
			} else if meta.path.is_ident("unique") {
				uniques.push(idents(&meta)?);
			} else if meta.path.is_ident("crate") {
				krate = Some(string(&meta)?.parse()?);
			} else {
				return Err(
					meta.error("unknown mixin option; expected `index`, `unique`, or `crate`")
				);
			}

			Ok(())
		});
		syn::parse::Parser::parse2(parser, tokens)?;

		Ok(Self {
			indexes,
			uniques,
			krate,
		})
	}
}

/// The client type a `public` table gets.
pub(crate) enum PublicName {
	/// `public`: `Public{Table}`.
	Default,
	/// `public = Profile`.
	Named(Ident),
}

/// Who may see a column.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Visibility {
	Public,
	/// Server only.
	Private,
	/// Server only, and redacted from `Debug`.
	Secret,
}

/// A column the ORM fills in itself.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timestamp {
	Created,
	Updated,
	Deleted,
}

/// A column the ORM fills in with who is acting.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActorMark {
	Created,
	Updated,
}

/// A default value from `#[db(default = ...)]`.
pub(crate) enum DefaultSpec {
	Lit(Lit),
	Negative(Lit),
	Now,
	Sql(LitStr),
}

/// The options on one field's `#[db(...)]` attributes.
#[expect(
	clippy::struct_excessive_bools,
	reason = "each flag is one attribute option"
)]
pub(crate) struct FieldArgs {
	pub key: bool,
	pub generate: Option<bool>,
	pub column: Option<LitStr>,
	pub unique: bool,
	pub unique_except: Option<Ident>,
	pub index: bool,
	pub visibility: Visibility,

	pub default: Option<DefaultSpec>,
	pub min_len: Option<Expr>,
	pub max_len: Option<Expr>,
	pub min: Option<Expr>,
	pub max: Option<Expr>,
	pub one_of: Vec<LitStr>,
	pub regex: Option<LitStr>,
	pub timestamp: Option<Timestamp>,
	pub on_delete: Option<Ident>,
	pub no_foreign_key: bool,
	/// The column is left out of the change log, for wide or secret data.
	pub live_skip: bool,
	pub renamed_from: Option<LitStr>,
	/// The field is a `#[db::mixin]` struct whose columns join the table.
	pub flatten: bool,
	/// What a flattened mixin's column names start with.
	pub prefix: Option<LitStr>,
	/// The column `save` checks and increments.
	pub version: bool,
	/// The method the linked table gets, listing the rows that point at it.
	pub backlink: Option<Ident>,
	/// What soft-deleting the linked row does to this one: `cascade`.
	pub on_soft_delete: Option<Ident>,
	/// The projections a `private` column is visible in: `in(SelfUser)`.
	pub projections: Vec<Ident>,
	/// The SQL the database computes the column from, on Postgres and on
	/// SQLite.
	pub generated: Option<(LitStr, LitStr)>,
	/// `created_by` or `updated_by`.
	pub actor: Option<ActorMark>,
	/// The field a slug column is made from: `slug(from = title)`.
	pub slug: Option<Ident>,
	/// How much a match in the column counts, when it's searched:
	/// `search(weight = A)`.
	pub search: Option<Ident>,
	/// A geospatial column's options: `geo(index, valid)`.
	pub geo: Option<GeoArgs>,
	/// A vector column's options: `vector(distance = …, index = …,
	/// source(…))`.
	pub vector: Option<VectorArgs>,
	/// `public`: a vector column clients may see, as vector columns aren't
	/// by default.
	pub public: Option<Span>,
}

/// `vector(distance = cosine, index = hnsw, source(title, body))` on a
/// vector column.
#[derive(Clone)]
pub struct VectorArgs {
	/// How it's measured: `l2`, `cosine`, `inner_product`, `l1`, `hamming`,
	/// or `jaccard`.
	pub distance: Option<Ident>,
	/// pgvector's index: `hnsw` or `ivfflat`.
	pub index: Option<Ident>,
	/// The text fields its vectors are made from.
	pub sources: Vec<Ident>,
	/// Where the option was written.
	pub span: Span,
}

/// `geo(index, valid)` on a geospatial column.
#[derive(Clone, Copy)]
pub struct GeoArgs {
	/// A spatial index: GiST on Postgres, an R*Tree on SQLite.
	pub index: bool,
	/// Shapes must be valid, as OGC defines it.
	pub valid: bool,
	/// Where the option was written.
	pub span: Span,
}

impl FieldArgs {
	pub fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
		let mut args = Self {
			key: false,
			generate: None,
			column: None,
			unique: false,
			unique_except: None,
			index: false,
			visibility: Visibility::Public,
			default: None,
			min_len: None,
			max_len: None,
			min: None,
			max: None,
			one_of: Vec::new(),
			regex: None,
			timestamp: None,
			on_delete: None,
			no_foreign_key: false,
			live_skip: false,
			renamed_from: None,
			flatten: false,
			prefix: None,
			version: false,
			backlink: None,
			on_soft_delete: None,
			projections: Vec::new(),
			generated: None,
			actor: None,
			slug: None,
			search: None,
			geo: None,
			vector: None,
			public: None,
		};

		for attr in attrs.iter().filter(|attr| is_db_attr(attr)) {
			attr.parse_nested_meta(|meta| args.option(&meta))?;
		}

		Ok(args)
	}

	/// The options that constrain a column's values: its default and the
	/// rules checked in Rust and in the database. Whether `meta` was one.
	fn rule(&mut self, meta: &ParseNestedMeta<'_>) -> syn::Result<bool> {
		let path = &meta.path;

		if path.is_ident("default") {
			self.default = Some(default(meta)?);
		} else if path.is_ident("min_len") {
			self.min_len = Some(meta.value()?.parse()?);
		} else if path.is_ident("max_len") {
			self.max_len = Some(meta.value()?.parse()?);
		} else if path.is_ident("min") {
			self.min = Some(meta.value()?.parse()?);
		} else if path.is_ident("max") {
			self.max = Some(meta.value()?.parse()?);
		} else if path.is_ident("one_of") {
			let content;
			parenthesized!(content in meta.input);
			self.one_of = Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?
				.into_iter()
				.collect();
		} else if path.is_ident("regex") {
			let pattern = string(meta)?;

			if let Err(error) = regex::Regex::new(&pattern.value()) {
				return Err(syn::Error::new(
					pattern.span(),
					format!("`regex` isn't a valid regular expression: {error}"),
				));
			}

			self.regex = Some(pattern);
		} else {
			return Ok(false);
		}

		Ok(true)
	}

	/// The options that index a column's contents: `search` and `geo`.
	/// Whether `meta` was one.
	fn index_option(&mut self, meta: &ParseNestedMeta<'_>) -> syn::Result<bool> {
		if meta.path.is_ident("search") {
			self.search = Some(search_weight(meta)?);
		} else if meta.path.is_ident("geo") {
			self.geo = Some(geo_args(meta)?);
		} else if meta.path.is_ident("vector") {
			self.vector = Some(vector_args(meta)?);
		} else {
			return Ok(false);
		}

		Ok(true)
	}

	/// The options that say who sees a column: `public`, `private`, and
	/// `secret`. Whether `meta` was one.
	fn visibility_option(&mut self, meta: &ParseNestedMeta<'_>) -> bool {
		if meta.path.is_ident("public") {
			self.public = Some(meta.path.span());
		} else if meta.path.is_ident("private") {
			self.visibility = Visibility::Private;
		} else if meta.path.is_ident("secret") {
			self.visibility = Visibility::Secret;
		} else {
			return false;
		}

		true
	}

	fn option(&mut self, meta: &ParseNestedMeta<'_>) -> syn::Result<()> {
		if self.rule(meta)? {
			return Ok(());
		}

		let path = &meta.path;

		if path.is_ident("key") {
			self.key = true;
		} else if path.is_ident("generate") {
			self.generate = Some(meta.value()?.parse::<syn::LitBool>()?.value);
		} else if path.is_ident("column") {
			self.column = Some(string(meta)?);
		} else if path.is_ident("unique") {
			self.unique = true;

			if meta.input.peek(syn::token::Paren) {
				meta.parse_nested_meta(|inner| {
					if inner.path.is_ident("except") {
						self.unique_except = Some(inner.value()?.parse()?);
						Ok(())
					} else {
						Err(inner.error("`unique` takes `except = field`, the soft-delete column"))
					}
				})?;
			}
		} else if path.is_ident("index") {
			self.index = true;
		} else if self.visibility_option(meta) {
		} else if path.is_ident("created_at") {
			self.timestamp = Some(Timestamp::Created);
		} else if path.is_ident("updated_at") {
			self.timestamp = Some(Timestamp::Updated);
		} else if path.is_ident("deleted_at") {
			self.timestamp = Some(Timestamp::Deleted);
		} else if path.is_ident("created_by") {
			self.actor = Some(ActorMark::Created);
		} else if path.is_ident("updated_by") {
			self.actor = Some(ActorMark::Updated);
		} else if path.is_ident("slug") {
			const HINT: &str = "`slug` takes `from = field`, the text it's made from, such as \
			                    `slug(from = title)`";
			let content;
			parenthesized!(content in meta.input);
			let key: Option<Ident> = content.parse()?;

			if key.as_ref().is_none_or(|key| key != "from") {
				return Err(meta.error(HINT));
			}

			content.parse::<Token![=]>()?;
			self.slug = Some(content.parse()?);

			if !content.is_empty() {
				return Err(content.error(HINT));
			}
		} else if self.index_option(meta)? {
		} else if path.is_ident("on_delete") {
			self.on_delete = Some(action(
				meta,
				&[
					"cascade",
					"set_null",
					"restrict",
					"no_action",
					"deferred_restrict",
				],
				"`on_delete` is one of `cascade`, `set_null`, `restrict`, `no_action`, or \
				 `deferred_restrict`",
			)?);
		} else if path.is_ident("no_foreign_key") {
			self.no_foreign_key = true;
		} else if path.is_ident("live") {
			let value: Ident = meta.value()?.parse()?;

			if value != "skip" {
				return Err(meta
					.error("`live` takes `skip`: the column is never copied into the change log"));
			}

			self.live_skip = true;
		} else if path.is_ident("renamed_from") {
			self.renamed_from = Some(string(meta)?);
		} else if path.is_ident("flatten") {
			self.flatten = true;
		} else if path.is_ident("prefix") {
			self.prefix = Some(string(meta)?);
		} else if path.is_ident("version") {
			self.version = true;
		} else if path.is_ident("backlink") {
			self.backlink = Some(meta.value()?.parse()?);
		} else if path.is_ident("generated") {
			self.generated = Some(dialect_sql(meta, "generated")?);
		} else if path.is_ident("in") {
			self.projections.extend(idents(meta)?);
		} else if path.is_ident("on_soft_delete") {
			self.on_soft_delete = Some(action(
				meta,
				&["cascade"],
				"`on_soft_delete` is `cascade`: soft-deleting the linked row soft-deletes this one",
			)?);
		} else {
			return Err(meta.error(
				"unknown column option; expected one of `key`, `generate`, `column`, `unique`, \
				 `index`, `private`, `secret`, `default`, `min_len`, `max_len`, `min`, `max`, \
				 `one_of`, `regex`, `created_at`, `updated_at`, `deleted_at`, `created_by`, \
				 `updated_by`, `version`, `slug`, `search`, `geo`, `vector`, `public`, \
				 `on_delete`, `on_soft_delete`, `backlink`, `no_foreign_key`, `live`, \
				 `renamed_from`, `flatten`, `prefix`, `in`, or `generated`",
			));
		}

		Ok(())
	}
}

/// `geo(index)`, `geo(valid)`, or both.
fn geo_args(meta: &ParseNestedMeta<'_>) -> syn::Result<GeoArgs> {
	const HINT: &str = "`geo` takes `index` (a spatial index), `valid` (shapes must be valid), or \
	                    both: `geo(index, valid)`";
	let mut args = GeoArgs {
		index: false,
		valid: false,
		span: meta.path.span(),
	};

	if !meta.input.peek(syn::token::Paren) {
		return Err(meta.error(HINT));
	}

	let content;
	parenthesized!(content in meta.input);

	for option in Punctuated::<Ident, Token![,]>::parse_terminated(&content)? {
		if option == "index" {
			args.index = true;
		} else if option == "valid" {
			args.valid = true;
		} else {
			return Err(syn::Error::new(option.span(), HINT));
		}
	}

	if !args.index && !args.valid {
		return Err(meta.error(HINT));
	}

	Ok(args)
}

/// `vector(distance = …, index = …, source(…))`, with at least one.
fn vector_args(meta: &ParseNestedMeta<'_>) -> syn::Result<VectorArgs> {
	const HINT: &str = "`vector` takes `distance = l2`, `cosine`, `inner_product`, `l1`, \
	                    `hamming`, or `jaccard`; `index = hnsw` or `ivfflat`; and `source(field, \
	                    …)`, the text its vectors are made from";
	const DISTANCES: [&str; 6] = ["l2", "cosine", "inner_product", "l1", "hamming", "jaccard"];
	let mut args = VectorArgs {
		distance: None,
		index: None,
		sources: Vec::new(),
		span: meta.path.span(),
	};

	let fork = meta.input.fork();
	let content;

	if !meta.input.peek(syn::token::Paren) || {
		parenthesized!(content in fork);
		content.is_empty()
	} {
		return Err(meta.error(HINT));
	}

	meta.parse_nested_meta(|inner| {
		if inner.path.is_ident("distance") {
			let distance: Ident = inner.value()?.parse()?;

			if !DISTANCES.contains(&distance.to_string().as_str()) {
				return Err(syn::Error::new(distance.span(), HINT));
			}

			args.distance = Some(distance);
		} else if inner.path.is_ident("index") {
			let index: Ident = inner.value()?.parse()?;

			if index != "hnsw" && index != "ivfflat" {
				return Err(syn::Error::new(
					index.span(),
					"`index` is `hnsw` or `ivfflat`, pgvector's indexes; SQLite searches exactly",
				));
			}

			args.index = Some(index);
		} else if inner.path.is_ident("source") {
			args.sources = idents(&inner)?;
		} else {
			return Err(inner.error(HINT));
		}

		Ok(())
	})?;

	if args.distance.is_none() && args.index.is_none() && args.sources.is_empty() {
		return Err(meta.error(HINT));
	}

	Ok(args)
}

/// Whether a field's type is a vector, `Option` or not: `Vector<N>`,
/// `HalfVector<N>`, `BitVector<N>`, or `SparseVector<N>`.
pub(crate) fn is_vector_type(ty: &syn::Type) -> bool {
	let syn::Type::Path(path) = ty else {
		return false;
	};
	let Some(last) = path.path.segments.last() else {
		return false;
	};

	if last.ident == "Option"
		&& let syn::PathArguments::AngleBracketed(arguments) = &last.arguments
		&& let Some(syn::GenericArgument::Type(inner)) = arguments.args.first()
	{
		return is_vector_type(inner);
	}

	["Vector", "HalfVector", "BitVector", "SparseVector"]
		.iter()
		.any(|name| last.ident == name)
}

impl FieldArgs {
	/// Makes a vector column private unless it says `public`, and refuses
	/// `public` anywhere else, or beside `private` or `secret`.
	pub(crate) fn default_visibility(&mut self, ty: &syn::Type) -> syn::Result<()> {
		let vector = self.vector.is_some() || is_vector_type(ty);

		match (self.public, vector) {
			(Some(span), false) => {
				Err(syn::Error::new(
					span,
					"`public` is for vector columns, which are private by default; other columns \
					 are public unless they're `private` or `secret`",
				))
			}

			(Some(span), true) if self.visibility != Visibility::Public => {
				Err(syn::Error::new(
					span,
					"a column is `public`, `private`, or `secret`, not more than one",
				))
			}

			(None, true) if self.visibility == Visibility::Public => {
				self.visibility = Visibility::Private;
				Ok(())
			}

			_ => Ok(()),
		}
	}
}

/// Whether an attribute is a `#[db(...)]` column option.
pub(crate) fn is_db_attr(attr: &Attribute) -> bool {
	attr.path().is_ident("db")
}

/// The text of `#[doc = "..."]` attributes, joined into one comment.
pub(crate) fn doc_comment(attrs: &[Attribute]) -> Option<String> {
	let lines: Vec<String> = attrs
		.iter()
		.filter(|attr| attr.path().is_ident("doc"))
		.filter_map(|attr| {
			match &attr.meta {
				syn::Meta::NameValue(pair) => {
					match &pair.value {
						Expr::Lit(syn::ExprLit {
							lit: Lit::Str(text),
							..
						}) => Some(text.value().trim().to_string()),
						_ => None,
					}
				}
				_ => None,
			}
		})
		.collect();

	if lines.is_empty() {
		None
	} else {
		Some(lines.join("\n").trim().to_string())
	}
}

fn string(meta: &ParseNestedMeta<'_>) -> syn::Result<LitStr> {
	meta.value()?.parse()
}

fn idents(meta: &ParseNestedMeta<'_>) -> syn::Result<Vec<Ident>> {
	let content;
	parenthesized!(content in meta.input);
	let idents: Vec<Ident> = Punctuated::<Ident, Token![,]>::parse_terminated(&content)?
		.into_iter()
		.collect();

	if idents.is_empty() {
		return Err(meta.error("name at least one field, such as `index(author_id, created_at)`"));
	}

	Ok(idents)
}

fn default(meta: &ParseNestedMeta<'_>) -> syn::Result<DefaultSpec> {
	if meta.input.peek(syn::token::Paren) {
		let mut sql = None;
		meta.parse_nested_meta(|inner| {
			if inner.path.is_ident("sql") {
				sql = Some(string(&inner)?);
				Ok(())
			} else {
				Err(inner.error("`default(...)` takes `sql = \"...\"`"))
			}
		})?;

		return sql
			.map(DefaultSpec::Sql)
			.ok_or_else(|| meta.error("`default(...)` needs `sql = \"...\"`"));
	}

	let value = meta.value()?;

	if value.peek(Token![-]) {
		value.parse::<Token![-]>()?;

		return Ok(DefaultSpec::Negative(value.parse()?));
	}

	if value.peek(Ident) {
		let ident: Ident = value.parse()?;

		return if ident == "now" {
			Ok(DefaultSpec::Now)
		} else {
			Err(syn::Error::new(
				ident.span(),
				"a default is a literal, `now`, or `default(sql = \"...\")`",
			))
		};
	}

	let lit: Lit = value.parse()?;

	match lit {
		Lit::Str(_) | Lit::Int(_) | Lit::Float(_) | Lit::Bool(_) => Ok(DefaultSpec::Lit(lit)),
		other => {
			Err(syn::Error::new(
				other.span(),
				"a default is a string, number, or boolean literal",
			))
		}
	}
}

/// `option = "sql"`, the same on both dialects, or `option(postgres = "…",
/// sqlite = "…")`, for `generated` and a view's `sql`.
fn dialect_sql(meta: &ParseNestedMeta<'_>, option: &str) -> syn::Result<(LitStr, LitStr)> {
	if !meta.input.peek(syn::token::Paren) {
		let sql = string(meta)?;

		return Ok((sql.clone(), sql));
	}

	let mut postgres = None;
	let mut sqlite = None;
	meta.parse_nested_meta(|inner| {
		if inner.path.is_ident("postgres") {
			postgres = Some(string(&inner)?);
		} else if inner.path.is_ident("sqlite") {
			sqlite = Some(string(&inner)?);
		} else {
			return Err(inner.error(format!(
				"`{option}(...)` takes `postgres = \"…\"` and `sqlite = \"…\"`"
			)));
		}

		Ok(())
	})?;

	match (postgres, sqlite) {
		(Some(postgres), Some(sqlite)) => Ok((postgres, sqlite)),
		_ => {
			Err(meta.error(format!(
				"`{option}(...)` needs both `postgres = \"…\"` and `sqlite = \"…\"`; for one \
				 expression on both, write `{option} = \"…\"`"
			)))
		}
	}
}

/// One of `allowed`, as an identifier: `on_delete = cascade`.
fn action(meta: &ParseNestedMeta<'_>, allowed: &[&str], expected: &str) -> syn::Result<Ident> {
	let action: Ident = meta.value()?.parse()?;

	if allowed.contains(&action.to_string().as_str()) {
		Ok(action)
	} else {
		Err(syn::Error::new(action.span(), expected))
	}
}

/// One `#[db::policy(…)]`: `[deny] action, …, |row, cx| rule[, message =
/// "…"]`.
pub(crate) struct PolicyArgs {
	pub deny: bool,
	/// `select`, `insert`, `update`, `delete`, or `all`.
	pub actions: Vec<Ident>,
	pub rule: syn::ExprClosure,
	pub message: Option<LitStr>,
}

impl syn::parse::Parse for PolicyArgs {
	fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
		const ACTIONS: &[&str] = &["select", "insert", "update", "delete", "all"];
		let mut deny = false;
		let mut actions = Vec::new();

		while !input.peek(Token![|]) && !input.peek(Token![||]) {
			let ident: Ident = input.parse()?;

			if actions.is_empty() && !deny && (ident == "deny" || ident == "allow") {
				deny = ident == "deny";
				continue;
			}

			if !ACTIONS.contains(&ident.to_string().as_str()) {
				return Err(syn::Error::new(
					ident.span(),
					"a policy governs `select`, `insert`, `update`, `delete`, or `all`, \
					 optionally after `deny`: `#[db::policy(deny insert, |row, cx| …)]`",
				));
			}

			actions.push(ident);
			input.parse::<Token![,]>()?;
		}

		if actions.is_empty() {
			return Err(input.error("name the actions the policy governs, such as `select,`"));
		}

		let rule: syn::ExprClosure = input.parse()?;

		if rule.inputs.len() != 2 {
			return Err(syn::Error::new_spanned(
				&rule.inputs,
				"a policy's rule takes the row's columns and the request: `|post, cx| …`",
			));
		}

		let mut message = None;

		if input.parse::<Option<Token![,]>>()?.is_some() && !input.is_empty() {
			let key: Ident = input.parse()?;

			if key != "message" {
				return Err(syn::Error::new(
					key.span(),
					"after the rule, a policy takes `message = \"…\"`",
				));
			}

			input.parse::<Token![=]>()?;
			message = Some(input.parse()?);
			input.parse::<Option<Token![,]>>()?;
		}

		Ok(Self {
			deny,
			actions,
			rule,
			message,
		})
	}
}

/// Whether an attribute is `#[db::policy(…)]` or an imported `#[policy(…)]`.
pub(crate) fn is_policy_attr(attr: &Attribute) -> bool {
	let segments = &attr.path().segments;
	segments.last().is_some_and(|last| last.ident == "policy")
		&& (segments.len() == 1 || segments.first().is_some_and(|first| first.ident == "db"))
}
