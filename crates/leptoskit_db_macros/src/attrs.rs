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
	pub krate: Option<Path>,
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
		let mut krate = None;

		let parser = syn::meta::parser(|meta| {
			if meta.path.is_ident("name") {
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
				let mut check_name = None;
				let mut sql = None;
				meta.parse_nested_meta(|inner| {
					if inner.path.is_ident("name") {
						check_name = Some(string(&inner)?);
					} else if inner.path.is_ident("sql") {
						sql = Some(string(&inner)?);
					} else {
						return Err(
							inner.error("`check` takes `name = \"...\"` and `sql = \"...\"`")
						);
					}
					Ok(())
				})?;
				let (Some(check_name), Some(sql)) = (check_name, sql) else {
					return Err(
						meta.error("`check` needs both `name = \"...\"` and `sql = \"...\"`")
					);
				};
				checks.push((check_name, sql));
			} else if meta.path.is_ident("renamed_from") {
				renamed_from = Some(string(&meta)?);
			} else if meta.path.is_ident("crate") {
				krate = Some(string(&meta)?.parse()?);
			} else {
				return Err(meta.error(
					"unknown table option; expected `name`, `namespace`, `public`, `index`, \
					 `unique`, `check`, `renamed_from`, or `crate`",
				));
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

		Ok(Self {
			name,
			namespace,
			public,
			indexes,
			uniques,
			checks,
			renamed_from,
			krate,
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
			renamed_from: None,
			flatten: false,
			prefix: None,
			version: false,
			backlink: None,
			on_soft_delete: None,
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
		} else if path.is_ident("private") {
			self.visibility = Visibility::Private;
		} else if path.is_ident("secret") {
			self.visibility = Visibility::Secret;
		} else if path.is_ident("created_at") {
			self.timestamp = Some(Timestamp::Created);
		} else if path.is_ident("updated_at") {
			self.timestamp = Some(Timestamp::Updated);
		} else if path.is_ident("deleted_at") {
			self.timestamp = Some(Timestamp::Deleted);
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
				 `one_of`, `regex`, `created_at`, `updated_at`, `deleted_at`, `version`, \
				 `on_delete`, `on_soft_delete`, `backlink`, `no_foreign_key`, `renamed_from`, \
				 `flatten`, or `prefix`",
			));
		}

		Ok(())
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
