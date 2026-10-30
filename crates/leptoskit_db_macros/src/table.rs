//! Expands `#[db::table]` into the server row, its insert type, its client
//! projection, and its schema description.
//!
//! A `#[db(flatten)]` field is a `#[db::mixin]` struct whose columns join the
//! table. The attribute can't see another struct's fields, so it asks: it
//! calls the macro the mixin declares under the same name, which calls
//! `table_continue!` back with the mixin's fields, once per flattened field.
//! Only then does the table expand, with every column known.

use proc_macro2::Span;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use quote::format_ident;
use quote::quote;
use quote::quote_spanned;
use syn::Attribute;
use syn::Fields;
use syn::GenericArgument;
use syn::Ident;
use syn::ItemStruct;
use syn::PathArguments;
use syn::Token;
use syn::Type;
use syn::parse::ParseStream;
use syn::spanned::Spanned;
use syn::visit_mut::VisitMut;

use crate::attrs::DefaultSpec;
use crate::attrs::FieldArgs;
use crate::attrs::LinkArgs;
use crate::attrs::LinkEnd;
use crate::attrs::MixinArgs;
use crate::attrs::PolicyArgs;
use crate::attrs::PublicName;
use crate::attrs::TableArgs;
use crate::attrs::Timestamp;
use crate::attrs::Visibility;
use crate::attrs::doc_comment;
use crate::attrs::is_db_attr;
use crate::attrs::is_policy_attr;

/// Derives the generated code handles itself; deriving them as well would
/// either collide or leak secrets.
pub(crate) const RESERVED_DERIVES: &[(&str, &str)] = &[
	(
		"Debug",
		"leptoskit generates `Debug` so that `secret` columns are redacted",
	),
	(
		"Serialize",
		"rows never cross the wire; return the generated public type (`Public…`) from server \
		 functions",
	),
	(
		"Deserialize",
		"rows never cross the wire; return the generated public type (`Public…`) from server \
		 functions",
	),
	(
		"JsonSchema",
		"the generated public type (`Public…`) already describes itself",
	),
];

struct Field {
	ident: Ident,
	ty: Type,
	vis: syn::Visibility,
	/// The field's attributes without `#[db(...)]`.
	attrs: Vec<Attribute>,
	docs: Option<String>,
	column: String,
	args: FieldArgs,
	/// The table this `Id<T>` column points at is this table.
	points_at_self: bool,
	key: bool,
	generated: bool,
	/// The columns of a `#[db(flatten)]` mixin field.
	mixin: Option<Mixin>,
}

/// A flattened mixin, as its macro described it.
struct Mixin {
	/// The mixin's type, which also names its macro.
	path: syn::Path,
	columns: Vec<MixinColumn>,
	indexes: Vec<Vec<Ident>>,
	uniques: Vec<Vec<Ident>>,
}

impl Mixin {
	/// The prefixed column of one of the mixin's fields.
	fn column_of(&self, ident: &Ident) -> Option<&MixinColumn> {
		self.columns.iter().find(|column| &column.ident == ident)
	}
}

struct MixinColumn {
	/// The mixin's field.
	ident: Ident,
	/// The field in `{Table}Columns`: the column name as an identifier.
	handle: Ident,
	column: String,
	args: FieldArgs,
	docs: Option<String>,
	/// Its `#[doc]` attributes, for the column handle.
	doc_attrs: Vec<Attribute>,
	/// The field's position, which names its type: `MixinField<N>`.
	position: usize,
}

/// One column, wherever it is declared: a field of the table, or a field of
/// a mixin the table flattens.
struct Column<'a> {
	/// The path to the value from the row: `handle` or `times.created_at`.
	access: TokenStream,
	handle: &'a Ident,
	name: &'a str,
	ty: TokenStream,
	span: Span,
	args: &'a FieldArgs,
	docs: Option<&'a str>,
	doc_attrs: Vec<&'a Attribute>,
	field: &'a Field,
	/// The mixin column, for columns a mixin declares.
	inner: Option<&'a MixinColumn>,
}

impl Column<'_> {
	/// The name validation errors use: the field, or the prefixed column for
	/// a mixin's field.
	fn label(&self) -> String {
		unraw(self.handle)
	}

	/// Whether an insert provides the column, rather than the ORM.
	fn is_provided(&self) -> bool {
		!self.field.generated && is_provided_args(self.args)
	}
}

pub(crate) fn expand(
	raw_args: &TokenStream,
	raw_item: &TokenStream,
	descriptions: &[TokenStream],
) -> syn::Result<TokenStream> {
	let mut args = TableArgs::parse(raw_args.clone())?;
	let mut item: ItemStruct = syn::parse2(raw_item.clone())?;
	let krate = crate::runtime(args.krate.as_ref());

	if !item.generics.params.is_empty() {
		return Err(syn::Error::new_spanned(
			&item.generics,
			"a table can't be generic",
		));
	}

	check_derives(&item.attrs, "table")?;

	// Policies are read here and never reach the row.
	let mut policies = Vec::new();
	let mut policy_errors: Option<syn::Error> = None;
	// An imported `#[policy]` is consumed here, so rustc never sees it used.
	let mut policy_import = None;
	item.attrs.retain(|attr| {
		if !is_policy_attr(attr) {
			return true;
		}

		if attr.path().segments.len() == 1 {
			policy_import = Some(attr.path().clone());
		}

		match attr.parse_args::<PolicyArgs>() {
			Ok(policy) => policies.push(policy),
			Err(error) => combine(&mut policy_errors, error),
		}

		false
	});

	if let Some(error) = policy_errors {
		return Err(error);
	}

	let name = item.ident.clone();
	let Fields::Named(named) = &mut item.fields else {
		return Err(syn::Error::new_spanned(
			&item.fields,
			"a table needs named fields: one per column",
		));
	};

	if let Some(link) = &args.link {
		let ends = inject_link_ends(&krate, link, named)?;
		let to = ends.1;

		// The key starts with the `from` column, so lookups from that end
		// use it; the other end needs its own index.
		if link.exclusive {
			args.uniques.push(vec![to]);
		} else {
			args.indexes.push(vec![to]);
		}
	}

	let mut fields = Vec::new();
	let mut errors: Option<syn::Error> = None;

	for field in &mut named.named {
		let mut ty = field.ty.clone();
		ReplaceSelf(&name).visit_type_mut(&mut ty);
		field.ty = ty.clone();

		let ident = field.ident.clone().expect("named fields have names");
		let args = match FieldArgs::parse(&field.attrs) {
			Ok(args) => args,
			Err(error) => {
				combine(&mut errors, error);
				continue;
			}
		};
		let points_at_self = id_target(&ty).is_some_and(|target| is_type_named(target, &name));

		fields.push(Field {
			column: args
				.column
				.as_ref()
				.map_or_else(|| unraw(&ident), syn::LitStr::value),
			docs: doc_comment(&field.attrs),
			attrs: field
				.attrs
				.iter()
				.filter(|attr| !is_db_attr(attr))
				.cloned()
				.collect(),
			ident,
			ty,
			vis: field.vis.clone(),
			args,
			points_at_self,
			key: false,
			generated: false,
			mixin: None,
		});
	}

	if let Some(errors) = errors {
		return Err(errors);
	}

	if args.link.is_some()
		&& let Some(field) = fields.iter().skip(2).find(|field| field.args.key)
	{
		return Err(syn::Error::new_spanned(
			&field.ident,
			"a link's key is its two ends; it can't have another",
		));
	}

	// Ask the next unresolved mixin for its fields.
	let flattened: Vec<&Field> = fields.iter().filter(|field| field.args.flatten).collect();

	if let Some(next) = flattened.get(descriptions.len()) {
		let path = mixin_path(next)?;
		return Ok(quote_spanned! { next.ty.span()=>
			#path! { (#krate::__private::table_continue) [#raw_args] [#raw_item] #({#descriptions})* }
		});
	}

	for (field, description) in fields
		.iter_mut()
		.filter(|field| field.args.flatten)
		.zip(descriptions)
	{
		field.mixin = Some(describe_mixin(field, description.clone())?);
	}

	resolve_keys(&name, &mut fields)?;

	let table = Table {
		krate: &krate,
		args: &args,
		name: &name,
		item: &item,
		fields: &fields,
		policies: &policies,
		policy_import: policy_import.as_ref(),
	};
	table.check_columns()?;
	table.check_links()?;
	Ok(table.generate())
}

/// Adds a link table's two ends as its first fields: required, cascading
/// links that together are the key. Returns their names.
fn inject_link_ends(
	krate: &TokenStream,
	link: &LinkArgs,
	named: &mut syn::FieldsNamed,
) -> syn::Result<(Ident, Ident)> {
	let column = |end: &LinkEnd| {
		end.column.as_ref().map_or_else(
			|| {
				let table = end
					.table
					.segments
					.last()
					.map(|segment| unraw(&segment.ident))
					.unwrap_or_default();
				let snake = convert_case::Converter::new()
					.from_case(convert_case::Case::Pascal)
					.to_case(convert_case::Case::Snake)
					.convert(table);
				format_ident!("{snake}_id", span = end.table.span())
			},
			|column| format_ident!("{}", column.value(), span = column.span()),
		)
	};
	let from = column(&link.from);
	let to = column(&link.to);

	if from == to {
		return Err(syn::Error::new_spanned(
			&link.to.table,
			format!(
				"both ends would be the column `{from}`; name them, such as `from_column = \
				 \"follower_id\", to_column = \"followed_id\"`"
			),
		));
	}

	for field in &named.named {
		if let Some(ident) = field
			.ident
			.as_ref()
			.filter(|ident| **ident == from || **ident == to)
		{
			return Err(syn::Error::new_spanned(
				ident,
				format!(
					"`{ident}` is one of the link's ends, which `#[db::link]` declares itself; \
					 declare only the link's own columns"
				),
			));
		}
	}

	let end = |ident: &Ident, table: &syn::Path, role: &str| {
		let doc = format!(" The `{}` row the link {role}.", quote!(#table));
		syn::parse::Parser::parse2(
			syn::Field::parse_named,
			quote_spanned! { table.span()=>
				#[doc = #doc]
				#[db(key, on_delete = cascade)]
				pub #ident: #krate::Id<#table>
			},
		)
	};
	named
		.named
		.insert(0, end(&to, &link.to.table, "points at")?);
	named
		.named
		.insert(0, end(&from, &link.from.table, "starts from")?);
	Ok((from, to))
}

/// Expands `table_continue!`: the table's arguments and item, and every
/// mixin description gathered so far.
pub(crate) fn expand_continued(input: TokenStream) -> syn::Result<TokenStream> {
	let mut groups = input.into_iter();
	let mut bracketed = |what: &str| {
		match groups.next() {
			Some(TokenTree::Group(group))
				if group.delimiter() == proc_macro2::Delimiter::Bracket =>
			{
				Ok(group.stream())
			}
			_ => {
				Err(syn::Error::new(
					Span::call_site(),
					format!("`table_continue!` expected the table's {what} in brackets"),
				))
			}
		}
	};
	let args = bracketed("arguments")?;
	let item = bracketed("item")?;
	let descriptions = groups
		.map(|tree| {
			match tree {
				TokenTree::Group(group) if group.delimiter() == proc_macro2::Delimiter::Brace => {
					Ok(group.stream())
				}
				other => {
					Err(syn::Error::new(
						other.span(),
						"`table_continue!` expected a mixin description in braces",
					))
				}
			}
		})
		.collect::<syn::Result<Vec<_>>>()?;

	expand(&args, &item, &descriptions)
}

/// The path of a flattened field's mixin, which also names its macro.
fn mixin_path(field: &Field) -> syn::Result<&syn::Path> {
	const HINT: &str = "#[db(flatten)] takes a `#[db::mixin]` struct, such as `db::Timestamps`";

	match &field.ty {
		Type::Path(path)
			if path.qself.is_none()
				&& path
					.path
					.segments
					.iter()
					.all(|segment| segment.arguments.is_empty()) =>
		{
			Ok(&path.path)
		}
		other => Err(syn::Error::new_spanned(other, HINT)),
	}
}

/// Reads the description a mixin's macro passed back: `[options]
/// [fields]`, each field its attributes and name.
fn describe_mixin(field: &Field, description: TokenStream) -> syn::Result<Mixin> {
	let path = mixin_path(field)?.clone();
	let prefix = field
		.args
		.prefix
		.as_ref()
		.map(syn::LitStr::value)
		.unwrap_or_default();
	let parser = |input: ParseStream<'_>| {
		let options;
		syn::bracketed!(options in input);
		let options: TokenStream = options.parse()?;
		let fields;
		syn::bracketed!(fields in input);
		let mut columns = Vec::new();

		while !fields.is_empty() {
			let attrs = fields.call(Attribute::parse_outer)?;
			let ident: Ident = fields.parse()?;
			fields.parse::<Token![;]>()?;
			let args = FieldArgs::parse(&attrs)?;
			let column = format!(
				"{prefix}{}",
				args.column
					.as_ref()
					.map_or_else(|| unraw(&ident), syn::LitStr::value)
			);
			columns.push(MixinColumn {
				handle: format_ident!("{prefix}{}", unraw(&ident), span = field.ident.span()),
				position: columns.len(),
				docs: doc_comment(&attrs),
				doc_attrs: attrs
					.iter()
					.filter(|attr| attr.path().is_ident("doc"))
					.cloned()
					.collect(),
				ident,
				column,
				args,
			});
		}

		Ok((options, columns))
	};
	let (options, columns) = syn::parse::Parser::parse2(parser, description)?;
	let options = MixinArgs::parse(options)?;

	Ok(Mixin {
		path,
		columns,
		indexes: options.indexes,
		uniques: options.uniques,
	})
}

pub(crate) fn check_derives(attrs: &[Attribute], kind: &str) -> syn::Result<()> {
	for attr in attrs.iter().filter(|attr| attr.path().is_ident("derive")) {
		attr.parse_nested_meta(|meta| {
			let derived = meta
				.path
				.segments
				.last()
				.map(|segment| segment.ident.to_string())
				.unwrap_or_default();

			if let Some((_, reason)) = RESERVED_DERIVES.iter().find(|(name, _)| *name == derived) {
				return Err(meta.error(format!("don't derive `{derived}` on a {kind}: {reason}")));
			}

			Ok(())
		})?;
	}

	Ok(())
}

/// Marks the key columns: `#[db(key)]` fields, or else a field named `id`.
/// A single `Id<Self>` key is generated on insert unless `generate = false`.
fn resolve_keys(name: &Ident, fields: &mut [Field]) -> syn::Result<()> {
	let explicit = fields.iter().any(|field| field.args.key);

	for field in fields.iter_mut() {
		field.key = if explicit {
			field.args.key
		} else {
			field.ident == "id"
		};

		if field.key && field.args.flatten {
			return Err(syn::Error::new_spanned(
				&field.ident,
				"a flattened mixin can't be the key",
			));
		}
	}

	let keys: Vec<&mut Field> = fields.iter_mut().filter(|field| field.key).collect();

	if keys.is_empty() {
		return Err(syn::Error::new_spanned(
			name,
			"a table needs a primary key: add `pub id: Id<Self>`, or mark the key fields with \
			 #[db(key)]",
		));
	}

	let single = keys.len() == 1;

	for field in keys {
		let wants = field
			.args
			.generate
			.unwrap_or(single && field.points_at_self);

		if wants && !(single && field.points_at_self) {
			return Err(syn::Error::new_spanned(
				&field.ty,
				"only a single `Id<Self>` key can be generated; set the key yourself with \
				 #[db(key, generate = false)]",
			));
		}

		field.generated = wants;
	}

	Ok(())
}

struct Table<'a> {
	krate: &'a TokenStream,
	args: &'a TableArgs,
	name: &'a Ident,
	item: &'a ItemStruct,
	fields: &'a [Field],
	policies: &'a [PolicyArgs],
	/// The path an imported `#[policy]` was written with, to mark it used.
	policy_import: Option<&'a syn::Path>,
}

impl Table<'_> {
	/// Every column, in declaration order, with a mixin's columns where the
	/// mixin is flattened.
	fn columns(&self) -> Vec<Column<'_>> {
		let krate = self.krate;
		let mut columns = Vec::new();

		for field in self.fields {
			let ident = &field.ident;

			match &field.mixin {
				None => {
					let ty = &field.ty;
					columns.push(Column {
						access: quote!(#ident),
						handle: ident,
						name: &field.column,
						ty: quote!(#ty),
						span: located(ty.span()),
						args: &field.args,
						docs: field.docs.as_deref(),
						doc_attrs: field
							.attrs
							.iter()
							.filter(|attr| attr.path().is_ident("doc"))
							.collect(),
						field,
						inner: None,
					});
				}
				Some(mixin) => {
					let path = &mixin.path;

					for inner in &mixin.columns {
						let inner_ident = &inner.ident;
						let position = inner.position;
						columns.push(Column {
							access: quote!(#ident.#inner_ident),
							handle: &inner.handle,
							name: &inner.column,
							ty: quote!(<#path as #krate::MixinField<#position>>::Type),
							span: located(field.ty.span()),
							args: &inner.args,
							docs: inner.docs.as_deref(),
							doc_attrs: inner.doc_attrs.iter().collect(),
							field,
							inner: Some(inner),
						});
					}
				}
			}
		}

		columns
	}

	fn check_columns(&self) -> syn::Result<()> {
		let columns = self.columns();
		let public = self.public_name();

		for (index, projection) in self.args.projections.iter().enumerate() {
			if self.args.projections[..index].contains(projection)
				|| public.as_ref() == Some(projection)
			{
				return Err(syn::Error::new_spanned(
					projection,
					format!("`{projection}` is already one of this table's client types"),
				));
			}
		}

		for (index, column) in columns.iter().enumerate() {
			if let Some(earlier) = columns[..index]
				.iter()
				.find(|earlier| earlier.name == column.name || earlier.handle == column.handle)
			{
				return Err(syn::Error::new_spanned(
					&column.field.ident,
					format!(
						"`{}` is already a column of this table{}; give the mixin a `prefix = \
						 \"...\"`",
						column.name,
						if std::ptr::eq(earlier.field, column.field) {
							""
						} else {
							", from another field"
						}
					),
				));
			}
		}

		for (marker, label) in [
			(Some(Timestamp::Created), "created_at"),
			(Some(Timestamp::Updated), "updated_at"),
			(Some(Timestamp::Deleted), "deleted_at"),
			(None, "version"),
		] {
			let marked: Vec<&Column<'_>> = columns
				.iter()
				.filter(|column| {
					match marker {
						Some(marker) => column.args.timestamp == Some(marker),
						None => column.args.version,
					}
				})
				.collect();

			if marked.len() > 1 {
				return Err(syn::Error::new_spanned(
					&marked[1].field.ident,
					format!("only one column can be #[db({label})]"),
				));
			}
		}

		for column in &columns {
			if let Some(except) = &column.args.unique_except
				&& self.column_named(column, except).is_none()
			{
				return Err(syn::Error::new_spanned(
					except,
					format!("`{}` has no field `{except}`", self.name),
				));
			}

			if column.field.key && (column.args.timestamp.is_some() || column.args.version) {
				return Err(syn::Error::new_spanned(
					&column.field.ident,
					"a key can't also be a timestamp or a version",
				));
			}

			if let Some((generated, _)) = &column.args.generated {
				let args = column.args;
				let conflict = [
					(column.field.key, "can't be the key"),
					(args.default.is_some(), "can't have a `default`"),
					(
						args.timestamp.is_some() || args.version,
						"can't be a timestamp or a version",
					),
					(args.flatten, "can't be a flattened mixin"),
					(
						args.min_len.is_some()
							|| args.max_len.is_some()
							|| args.min.is_some()
							|| args.max.is_some()
							|| !args.one_of.is_empty()
							|| args.regex.is_some(),
						"can't have rules such as `min_len`; put them on the columns it's \
						 computed from",
					),
					(
						column.inner.is_none() && id_target(&column.field.ty).is_some(),
						"can't be a link",
					),
				]
				.into_iter()
				.find_map(|(conflicts, problem)| conflicts.then_some(problem));

				if let Some(problem) = conflict {
					return Err(syn::Error::new_spanned(
						generated,
						format!(
							"a generated column {problem}: the database computes its value from \
							 the row's other columns"
						),
					));
				}
			}

			for audience in &column.args.projections {
				let problem = if column.args.visibility == Visibility::Secret {
					Some(
						"a `secret` column never leaves the server, so it can't be in a projection",
					)
				} else if column.args.visibility == Visibility::Public {
					Some(
						"a public column is in every projection already; `in(...)` names the \
						 projections a `private` column is shown in",
					)
				} else if !self.args.projections.contains(audience) {
					Some("declare it on the table first: `projection(...)`")
				} else {
					None
				};

				if let Some(problem) = problem {
					return Err(syn::Error::new_spanned(
						audience,
						format!("`{audience}`: {problem}"),
					));
				}
			}

			if column.inner.is_none() && column.args.prefix.is_some() && !column.args.flatten {
				return Err(syn::Error::new_spanned(
					&column.field.ident,
					"`prefix` names a flattened mixin's columns; add `flatten`",
				));
			}
		}

		Ok(())
	}

	/// Links get methods named after their fields, so the names must be free.
	fn check_links(&self) -> syn::Result<()> {
		const GENERATED: &[&str] = &[
			"query", "update", "find", "get", "save", "delete", "destroy",
		];
		let mut names: Vec<String> = Vec::new();

		for (field, _) in self.link_fields() {
			let link = unraw(&link_name(&field.ident));

			if GENERATED.contains(&link.as_str()) {
				return Err(syn::Error::new_spanned(
					&field.ident,
					format!(
						"`{}` would get the accessor `{link}()`, which every table already has; \
						 rename the field",
						field.ident
					),
				));
			}

			if names.contains(&link) {
				return Err(syn::Error::new_spanned(
					&field.ident,
					format!("two links would both get the accessor `{link}()`; rename one"),
				));
			}

			names.push(link);
		}

		for field in self
			.fields
			.iter()
			.filter(|field| field.args.backlink.is_some())
		{
			if !self
				.link_fields()
				.any(|(link, _)| std::ptr::eq(link, field))
			{
				return Err(syn::Error::new_spanned(
					&field.ident,
					"`backlink` names a method on the table an `Id<T>` column points at; this \
					 field isn't one",
				));
			}
		}

		for field in self
			.fields
			.iter()
			.filter(|field| field.args.on_soft_delete.is_some())
		{
			if !self
				.link_fields()
				.any(|(link, _)| std::ptr::eq(link, field))
			{
				return Err(syn::Error::new_spanned(
					&field.ident,
					"`on_soft_delete` follows an `Id<T>` column; this field isn't one",
				));
			}

			if !self
				.columns()
				.iter()
				.any(|column| column.args.timestamp == Some(Timestamp::Deleted))
			{
				return Err(syn::Error::new_spanned(
					&field.ident,
					format!(
						"`on_soft_delete = cascade` soft-deletes `{}` rows, so the table needs a \
						 `#[db(deleted_at)]` column or `db::SoftDelete`",
						self.name
					),
				));
			}
		}

		Ok(())
	}

	/// The column `ident` names, seen from `from`: a mixin's own field, or
	/// else a field of the table.
	fn column_named(&self, from: &Column<'_>, ident: &Ident) -> Option<String> {
		if let Some(mixin) = from.field.mixin.as_ref()
			&& let Some(inner) = mixin.column_of(ident)
		{
			return Some(inner.column.clone());
		}

		self.table_column(ident)
	}

	/// The column a table field or flattened column handle names.
	fn table_column(&self, ident: &Ident) -> Option<String> {
		self.columns()
			.into_iter()
			.find(|column| column.handle == ident)
			.map(|column| column.name.to_string())
	}

	fn generate(&self) -> TokenStream {
		let krate = self.krate;
		let row = self.row();
		let debug = self.debug();
		let table_impl = self.table_impl();
		let new = self.new_type();
		let columns = self.columns_type();
		let methods = self.methods();
		let public = self.public_type();
		let has_mixins = self.has_mixins();
		let policy_import = self.policy_import.map(|path| {
			quote! {
				const _: () = {
					#[allow(unused_imports)]
					use #path as _;
				};
			}
		});
		let links = self.links();
		let link = self.link_impl();
		let public_columns = self.public_columns();
		let projections = self.projections();
		let name = self.name;
		let vis = &self.item.vis;
		let docs = self
			.item
			.attrs
			.iter()
			.filter(|attr| attr.path().is_ident("doc"));

		quote! {
			#krate::__server! {
				#row
				#debug
				#columns
				#table_impl
				#new
				#methods
				#has_mixins
				#links
				#link
				#public_columns
				#krate::__private::inventory::submit! {
					#krate::__private::TableEntry::of::<#name>()
				}
			}

			#krate::__client! {
				#(#docs)*
				///
				/// On the client this is only a marker: it names the table in
				/// `Id<…>`, and has no rows to construct.
				#vis enum #name {}
			}

			#public
			#(#projections)*
			#policy_import
		}
	}

	fn row(&self) -> TokenStream {
		let item = self.item;
		let attrs = &item.attrs;
		let vis = &item.vis;
		let name = self.name;
		let fields = self.fields.iter().map(|field| {
			let Field {
				ident,
				ty,
				vis,
				attrs,
				..
			} = field;
			quote! { #(#attrs)* #vis #ident: #ty }
		});

		quote! {
			#(#attrs)*
			#vis struct #name {
				#(#fields,)*
			}
		}
	}

	fn debug(&self) -> TokenStream {
		let name = self.name;
		let label = name.to_string();
		let fields = self.fields.iter().map(|field| {
			let ident = &field.ident;
			let label = ident.to_string();

			if field.args.visibility == Visibility::Secret {
				quote! { .field(#label, &::core::format_args!("<secret>")) }
			} else {
				quote! { .field(#label, &self.#ident) }
			}
		});

		quote! {
			impl ::core::fmt::Debug for #name {
				fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
					formatter.debug_struct(#label) #(#fields)* .finish()
				}
			}
		}
	}

	/// The column `ident` names among the table's own fields and flattened
	/// columns, or the identifier itself.
	fn column_of(&self, ident: &Ident) -> String {
		self.table_column(ident)
			.unwrap_or_else(|| ident.to_string())
	}

	/// The column carrying a marker, as an `Option<&str>` expression.
	fn marker(&self, marked: impl Fn(&FieldArgs) -> bool) -> TokenStream {
		option_str(
			self.columns()
				.into_iter()
				.find(|column| marked(column.args))
				.map(|column| column.name.to_string()),
		)
	}

	/// Statements run on the column carrying a marker, if there is one.
	fn on_marker(
		&self,
		marked: impl Fn(&FieldArgs) -> bool,
		statement: impl Fn(&TokenStream) -> TokenStream,
	) -> Option<TokenStream> {
		self.columns()
			.into_iter()
			.find(|column| marked(column.args))
			.map(|column| {
				let span = column.span;
				let body = statement(&column.access);
				quote_spanned! { span=> #body }
			})
	}

	fn table_impl(&self) -> TokenStream {
		let krate = self.krate;
		let name = self.name;
		let table_name = &self.args.name;
		let namespace = option_str(self.args.namespace.as_ref().map(syn::LitStr::value));
		let all = self.columns();
		let columns = all.iter().map(|column| column.name);
		let keys = self
			.fields
			.iter()
			.filter(|field| field.key)
			.map(|field| &field.column);
		let created = self.marker(|args| args.timestamp == Some(Timestamp::Created));
		let updated = self.marker(|args| args.timestamp == Some(Timestamp::Updated));
		let deleted = self.marker(|args| args.timestamp == Some(Timestamp::Deleted));
		let version = self.marker(|args| args.version);
		let pack = option_str(self.args.pack.as_ref().map(syn::LitStr::value));
		let definition = self.definition();
		let columns_name = self.columns_name();
		let column_handles = all.iter().map(|column| {
			let handle = column.handle;
			let name = column.name;
			quote! { #handle: #krate::Col::new(#name) }
		});
		// The database computes generated columns, so the row holds only a
		// placeholder for them until it's written.
		let validate = validations(
			krate,
			all.iter().filter(|column| column.args.generated.is_none()),
		);
		let generated = all
			.iter()
			.filter(|column| column.args.generated.is_some())
			.map(|column| column.name);
		let to_values = all.iter().map(|column| {
			let access = &column.access;
			quote_spanned! { column.span=> #krate::Column::to_value(&self.#access)? }
		});
		let from_values = self.fields.iter().map(|field| {
			let ident = &field.ident;

			match &field.mixin {
				None => {
					let column = &field.column;
					quote! { #ident: #krate::__private::decode_next(&mut values, #table_name, #column)? }
				}
				Some(mixin) => {
					let path = &mixin.path;
					let inner = mixin.columns.iter().map(|inner| {
						let inner_ident = &inner.ident;
						let column = &inner.column;
						quote! { #inner_ident: #krate::__private::decode_next(&mut values, #table_name, #column)? }
					});
					quote! { #ident: #path { #(#inner),* } }
				}
			}
		});
		let touch = self.on_marker(
			|args| args.timestamp == Some(Timestamp::Updated),
			|access| quote! { self.#access = #krate::now(); },
		);
		let mark_deleted = self.on_marker(
			|args| args.timestamp == Some(Timestamp::Deleted),
			|access| quote! { self.#access = ::core::option::Option::Some(#krate::now()); },
		);
		let policies = self.policies_fn();
		let bump_version =
			self.on_marker(|args| args.version, |access| quote! { self.#access += 1; });

		quote! {
			impl #krate::Table for #name {
				const NAME: &'static str = #table_name;
				const NAMESPACE: ::core::option::Option<&'static str> = #namespace;
				const COLUMNS: &'static [&'static str] = &[#(#columns),*];
				const KEY: &'static [&'static str] = &[#(#keys),*];
				const CREATED_AT: ::core::option::Option<&'static str> = #created;
				const UPDATED_AT: ::core::option::Option<&'static str> = #updated;
				const DELETED_AT: ::core::option::Option<&'static str> = #deleted;
				const VERSION: ::core::option::Option<&'static str> = #version;
				const PACK: ::core::option::Option<&'static str> = #pack;
				const GENERATED: &'static [&'static str] = &[#(#generated),*];
				type Columns = #columns_name;

				fn columns() -> &'static #columns_name {
					static COLUMNS: #columns_name = #columns_name { #(#column_handles),* };
					&COLUMNS
				}

				fn validate(&self) -> ::core::result::Result<(), #krate::ValidationErrors> {
					#validate
				}

				fn definition() -> #krate::__private::schema::Table {
					#definition
				}

				fn to_values(&self) -> ::core::result::Result<::std::vec::Vec<#krate::Value>, #krate::EncodeError> {
					::core::result::Result::Ok(::std::vec![#(#to_values),*])
				}

				fn from_values(values: ::std::vec::Vec<#krate::Value>) -> ::core::result::Result<Self, #krate::DecodeError> {
					let mut values = values.into_iter();
					::core::result::Result::Ok(Self { #(#from_values),* })
				}

				fn touch(&mut self) {
					#touch
				}

				fn mark_deleted(&mut self) {
					#mark_deleted
				}

				fn bump_version(&mut self) {
					#bump_version
				}

				#policies
			}
		}
	}

	/// `Table::policies`, from the `#[db::policy]` attributes. Each rule is a
	/// closure that captures nothing, so it becomes a plain function.
	fn policies_fn(&self) -> Option<TokenStream> {
		if self.policies.is_empty() {
			return None;
		}

		let krate = self.krate;
		let name = self.name;
		let policies = self.policies.iter().map(|policy| {
			let actions: Vec<TokenStream> = policy
				.actions
				.iter()
				.map(|action| {
					if action == "all" {
						quote! { #krate::Action::Select, #krate::Action::Insert, #krate::Action::Update, #krate::Action::Delete }
					} else {
						let variant = format_ident!(
							"{}",
							convert_case::Converter::new()
								.from_case(convert_case::Case::Snake)
								.to_case(convert_case::Case::Pascal)
								.convert(action.to_string()),
							span = action.span()
						);
						quote! { #krate::Action::#variant }
					}
				})
				.collect();
			let deny = policy.deny;
			let rule = &policy.rule;
			let message = option_tokens(policy.message.as_ref().map(|message| quote!(#message)));
			quote! {
				#krate::Policy {
					actions: &[#(#actions),*],
					deny: #deny,
					rule: #rule,
					message: #message,
				}
			}
		});

		Some(quote! {
			fn policies() -> &'static [#krate::Policy<Self>] {
				static POLICIES: &[#krate::Policy<#name>] = &[#(#policies),*];
				POLICIES
			}
		})
	}

	fn definition(&self) -> TokenStream {
		let krate = self.krate;
		let table_name = &self.args.name;
		let namespace = option_str(self.args.namespace.as_ref().map(syn::LitStr::value));
		let comment = option_str(doc_comment(&self.item.attrs));
		let renamed_from = option_str(self.args.renamed_from.as_ref().map(syn::LitStr::value));
		let mut calls: Vec<TokenStream> = self
			.columns()
			.iter()
			.flat_map(|column| self.column_calls(column))
			.collect();

		let keys = self
			.fields
			.iter()
			.filter(|field| field.key)
			.map(|field| &field.column);
		calls.push(quote! { table.key(&[#(#keys),*]); });

		for columns in &self.args.indexes {
			let columns = columns.iter().map(|ident| self.column_of(ident));
			calls.push(quote! { table.index(&[#(#columns),*]); });
		}

		for columns in &self.args.uniques {
			let columns = columns.iter().map(|ident| self.column_of(ident));
			calls.push(quote! { table.unique(&[#(#columns),*], ::core::option::Option::None); });
		}

		// The rules a mixin declares over its own fields.
		for mixin in self.fields.iter().filter_map(|field| field.mixin.as_ref()) {
			let column = |ident: &Ident| {
				mixin
					.column_of(ident)
					.map_or_else(|| ident.to_string(), |inner| inner.column.clone())
			};

			for columns in &mixin.indexes {
				let columns = columns.iter().map(column);
				calls.push(quote! { table.index(&[#(#columns),*]); });
			}

			for columns in &mixin.uniques {
				let columns = columns.iter().map(column);
				calls
					.push(quote! { table.unique(&[#(#columns),*], ::core::option::Option::None); });
			}
		}

		for (name, sql) in &self.args.checks {
			calls.push(quote! { table.sql_check(#name, #sql); });
		}

		quote! {
			let mut table = #krate::__private::TableBuilder::new(#namespace, #table_name, #comment, #renamed_from);
			#(#calls)*
			table.build()
		}
	}

	/// The builder calls for one column: the column itself, then each rule
	/// and index it declares, then its foreign key.
	fn column_calls(&self, column: &Column<'_>) -> Vec<TokenStream> {
		let krate = self.krate;
		let name = column.name;
		let ty = &column.ty;
		let span = column.span;
		let args = column.args;
		let default = option_tokens(
			args.default
				.as_ref()
				.map(|default| default_value(krate, default)),
		);
		let comment = option_str(column.docs.map(ToString::to_string));
		let renamed_from = option_str(args.renamed_from.as_ref().map(syn::LitStr::value));
		let mut calls = vec![quote_spanned! { span=>
			table.column::<#ty>(#name, #default, #comment, #renamed_from);
		}];

		if let Some((postgres, sqlite)) = &args.generated {
			calls.push(quote! { table.generated(#name, #postgres, #sqlite); });
		}

		if args.unique {
			let except = option_str(
				args.unique_except
					.as_ref()
					.and_then(|except| self.column_named(column, except)),
			);
			calls.push(quote! { table.unique(&[#name], #except); });
		}

		if args.index {
			calls.push(quote! { table.index(&[#name]); });
		}

		if args.min_len.is_some() || args.max_len.is_some() {
			let min = option_u32(args.min_len.as_ref());
			let max = option_u32(args.max_len.as_ref());
			calls.push(quote! { table.length(#name, #min, #max); });
		}

		if args.min.is_some() || args.max.is_some() {
			let min = option_f64(args.min.as_ref());
			let max = option_f64(args.max.as_ref());
			calls.push(quote! { table.range(#name, #min, #max); });
		}

		if !args.one_of.is_empty() {
			let values = &args.one_of;
			calls.push(quote! { table.one_of(#name, &[#(#values),*]); });
		}

		if let Some(pattern) = &args.regex {
			calls.push(quote! { table.pattern(#name, #pattern); });
		}

		// `Id<T>` columns reference another table, and a table's own
		// `Id<Self>` key references nothing. A mixin's field types aren't
		// visible here, so its columns ask their type at run time.
		let field = column.field;
		let references = match column.inner {
			Some(_) => true,
			None => id_target(&field.ty).is_some() && !(field.key && field.points_at_self),
		};

		if references && !args.no_foreign_key {
			// Postgres can't defer `RESTRICT`; a deferred `NO ACTION` is the
			// same check, made at commit.
			let deferred = args
				.on_delete
				.as_ref()
				.is_some_and(|action| action == "deferred_restrict");
			let action = args.on_delete.as_ref().map(|action| {
				let action = if deferred {
					"no_action".to_string()
				} else {
					action.to_string()
				};
				let variant = format_ident!(
					"{}",
					convert_case::Converter::new()
						.from_case(convert_case::Case::Snake)
						.to_case(convert_case::Case::Pascal)
						.convert(action)
				);
				quote! { #krate::__private::schema::ReferentialAction::#variant }
			});
			let action = option_tokens(action);
			calls.push(
				quote_spanned! { span=> table.foreign_key::<#ty>(#name, #action, #deferred); },
			);
		}

		calls
	}

	fn columns_name(&self) -> Ident {
		format_ident!("{}Columns", self.name)
	}

	/// One typed handle per column, for filters and orderings.
	fn columns_type(&self) -> TokenStream {
		let krate = self.krate;
		let name = self.name;
		let columns = self.columns_name();
		let vis = &self.item.vis;
		let fields = self.columns().into_iter().map(|column| {
			let handle = column.handle;
			let ty = &column.ty;
			let docs = &column.doc_attrs;
			quote! { #(#docs)* pub #handle: #krate::Col<#name, #ty> }
		});
		let doc = format!(
			" The columns of [`{name}`], for filters and orderings: `{name}::query().filter(|c| \
			 …)`."
		);

		quote! {
			#[doc = #doc]
			#vis struct #columns {
				#(#fields,)*
			}
		}
	}

	/// The queries and writes a table offers, delegating to the runtime.
	fn methods(&self) -> TokenStream {
		let krate = self.krate;
		// A pack's own API writes to its tables; everyone else only reads them.
		let write_vis = if self.args.pack.is_some() {
			quote!(pub(crate))
		} else {
			quote!(pub)
		};
		let name = self.name;
		let new = self.new_name();
		let columns = self.columns_name();
		let keys: Vec<&Field> = self.fields.iter().filter(|field| field.key).collect();
		let by_key = match keys.as_slice() {
			[key] => {
				let ident = &key.ident;
				let ty = &key.ty;
				quote! {
					/// The row with this key, if it exists and isn't soft-deleted.
					pub async fn find(
						connection: &impl #krate::Connection,
						key: #ty,
					) -> #krate::Result<::core::option::Option<Self>> {
						Self::query().filter(|columns| columns.#ident.eq(key)).first(connection).await
					}

					/// The row with this key, or [`DbError::NotFound`](#krate::DbError::NotFound).
					pub async fn get(connection: &impl #krate::Connection, key: #ty) -> #krate::Result<Self> {
						Self::query().filter(|columns| columns.#ident.eq(key)).one(connection).await
					}
				}
			}
			_ => quote! {},
		};

		quote! {
			impl #name {
				/// A query on this table.
				pub fn query() -> #krate::Query<Self> {
					#krate::Query::new()
				}

				/// An update of many rows at once.
				#write_vis fn update() -> #krate::Update<Self> {
					#krate::Update::new()
				}

				#by_key

				/// Writes every column, matched by the key, and bumps `updated_at`.
				#write_vis async fn save(&mut self, connection: &impl #krate::Connection) -> #krate::Result<()> {
					#krate::__private::write::save(connection, self).await
				}

				/// Deletes the row: soft-deletes it when the table has a
				/// `#[db(deleted_at)]` column.
				#write_vis async fn delete(&mut self, connection: &impl #krate::Connection) -> #krate::Result<()> {
					#krate::__private::write::delete(connection, self).await
				}

				/// Removes the row for good.
				#write_vis async fn destroy(self, connection: &impl #krate::Connection) -> #krate::Result<()> {
					#krate::__private::write::destroy(connection, &self).await
				}
			}

			impl #new {
				/// Inserts the row and returns it as stored.
				#write_vis async fn insert(self, connection: &impl #krate::Connection) -> #krate::Result<#name> {
					#krate::__private::write::insert(connection, self.into_row()).await
				}

				/// Inserts the row, or updates the existing row whose `target`
				/// columns match, keeping its key and `created_at`.
				#write_vis async fn upsert<C: #krate::ConflictTarget>(
					self,
					connection: &impl #krate::Connection,
					target: impl ::core::ops::FnOnce(&#columns) -> C,
				) -> #krate::Result<#name> {
					let target = #krate::ConflictTarget::columns(&target(<#name as #krate::Table>::columns()));
					#krate::__private::write::upsert(connection, self.into_row(), &target).await
				}
			}
		}
	}

	/// The table's links: its own `Id<T>` columns, skipping the key that
	/// names the row itself. A mixin's columns have no accessors, since
	/// their types aren't visible here.
	fn link_fields(&self) -> impl Iterator<Item = (&Field, &Type)> {
		self.fields
			.iter()
			.filter(|field| field.mixin.is_none() && !(field.key && field.points_at_self))
			.filter_map(|field| id_target(&field.ty).map(|target| (field, target)))
	}

	/// For each link: an accessor that loads the row it points at, a `by_…`
	/// query for the rows pointing at one, and, with `backlink = …`, a method
	/// on the linked table listing the rows that point at it. Also `HasId`,
	/// which a table keyed by `Id<Self>` needs to be linked to.
	fn links(&self) -> TokenStream {
		let krate = self.krate;
		let name = self.name;
		let mut methods = Vec::new();
		let mut backlinks = Vec::new();

		for (field, target) in self.link_fields() {
			let ident = &field.ident;
			let link = link_name(ident);
			let by = format_ident!("by_{}", unraw(&link), span = ident.span());
			let accessor = if generic_argument(&field.ty, "Option").is_some() {
				let doc = format!(
					" The row `{ident}` points at, if it's set and the row exists and isn't \
					 soft-deleted."
				);
				quote! {
					#[doc = #doc]
					pub async fn #link(&self, connection: &impl #krate::Connection) -> #krate::Result<::core::option::Option<#target>> {
						#krate::__private::link::optional(connection, self.#ident).await
					}
				}
			} else {
				let doc = format!(
					" The row `{ident}` points at. A soft-deleted row is `DbError::NotFound`."
				);
				quote! {
					#[doc = #doc]
					pub async fn #link(&self, connection: &impl #krate::Connection) -> #krate::Result<#target> {
						#krate::__private::link::one(connection, self.#ident).await
					}
				}
			};
			let by_doc = format!(" The rows whose `{ident}` points at `id`.");
			let via = format_ident!("__via_{}", unraw(&link), span = ident.span());
			let column = &field.column;
			let relation = if generic_argument(&field.ty, "Option").is_some() {
				quote! {
					pub fn #via() -> #krate::__private::shape::Relation<Self, #target, #krate::__private::shape::MaybeOne> {
						#krate::__private::shape::Relation::forward_optional(#column)
					}
				}
			} else {
				quote! {
					pub fn #via() -> #krate::__private::shape::Relation<Self, #target, #krate::__private::shape::One> {
						#krate::__private::shape::Relation::forward(#column)
					}
				}
			};
			methods.push(quote! {
				#accessor

				#[doc = #by_doc]
				pub fn #by(id: #krate::Id<#target>) -> #krate::Query<Self> {
					Self::query().filter(|columns| columns.#ident.eq(id))
				}

				/// The link, for shapes to follow.
				#[doc(hidden)]
				#relation
			});

			if field.args.on_soft_delete.is_some() {
				let column = &field.column;
				backlinks.push(quote! {
					#krate::__private::inventory::submit! {
						#krate::__private::SoftCascade::of::<#name, #target>(#column)
					}
				});
			}

			if let Some(backlink) = &field.args.backlink {
				let doc = format!(" The [`{name}`] rows whose `{ident}` points at this row.");
				// The linked table must be in this crate: Rust only allows
				// inherent methods there.
				let via = format_ident!("__via_{}", unraw(backlink), span = backlink.span());
				let column = &field.column;
				backlinks.push(quote_spanned! { backlink.span()=>
					impl #target {
						#[doc = #doc]
						pub fn #backlink(&self) -> #krate::Query<#name> {
							#name::#by(<Self as #krate::HasId>::id(self))
						}

						/// The backlink, for shapes to follow.
						#[doc(hidden)]
						pub fn #via() -> #krate::__private::shape::Relation<Self, #name, #krate::__private::shape::Many> {
							#krate::__private::shape::Relation::backward(#column)
						}
					}
				});
			}
		}

		let has_id = match self
			.fields
			.iter()
			.filter(|field| field.key)
			.collect::<Vec<_>>()
			.as_slice()
		{
			[key] if key.points_at_self => {
				let ident = &key.ident;
				Some(quote! {
					impl #krate::HasId for #name {
						fn id(&self) -> #krate::Id<Self> {
							self.#ident
						}
					}
				})
			}
			_ => None,
		};

		let methods = (!methods.is_empty()).then(|| {
			quote! {
				impl #name {
					#(#methods)*
				}
			}
		});

		quote! {
			#has_id
			#methods
			#(#backlinks)*
		}
	}

	/// `HasMixin` for each mixin flattened without a prefix, so generic code
	/// can reach it.
	fn has_mixins(&self) -> TokenStream {
		let krate = self.krate;
		let name = self.name;
		let impls = self.fields.iter().filter_map(|field| {
			let mixin = field.mixin.as_ref()?;

			if field.args.prefix.is_some() {
				return None;
			}

			let ident = &field.ident;
			let path = &mixin.path;
			Some(quote! {
				impl #krate::HasMixin<#path> for #name {
					fn mixin(&self) -> &#path {
						&self.#ident
					}

					fn mixin_mut(&mut self) -> &mut #path {
						&mut self.#ident
					}
				}
			})
		});

		quote! { #(#impls)* }
	}

	fn new_name(&self) -> Ident {
		format_ident!("New{}", self.name)
	}

	/// The fields an insert provides, each with its declaration in the
	/// insert type: every column except the generated key, the timestamps,
	/// and the version. A flattened mixin is provided whole when the insert
	/// provides any of its columns.
	fn provided_fields(&self) -> Vec<(&Field, TokenStream)> {
		let krate = self.krate;

		self.fields
			.iter()
			.filter_map(|field| {
				let Field { ident, ty, vis, .. } = field;
				let docs = field
					.attrs
					.iter()
					.filter(|attr| attr.path().is_ident("doc"));

				let declaration = match &field.mixin {
					None if is_provided(field) => quote! { #(#docs)* #vis #ident: #ty },
					None => return None,
					Some(mixin) => {
						let path = &mixin.path;

						if !mixin
							.columns
							.iter()
							.any(|inner| is_provided_args(&inner.args))
						{
							return None;
						}

						quote! { #(#docs)* #vis #ident: <#path as #krate::Mixin>::New }
					}
				};

				Some((field, declaration))
			})
			.collect()
	}

	/// For a `#[db::link]` table: `Link`, the properties a new link takes,
	/// and the methods on both ends that follow it.
	fn link_impl(&self) -> TokenStream {
		let Some(link) = &self.args.link else {
			return quote! {};
		};
		let krate = self.krate;
		let name = self.name;
		let new = self.new_name();
		let vis = &self.item.vis;
		// `inject_link_ends` put the ends first.
		let (from, to) = (&self.fields[0], &self.fields[1]);
		let (from_ident, to_ident) = (&from.ident, &to.ident);
		let (from_column, to_column) = (&from.column, &to.column);
		let (from_table, to_table) = (&link.from.table, &link.to.table);
		let properties: Vec<(&Field, TokenStream)> = self
			.provided_fields()
			.into_iter()
			.filter(|(field, _)| !std::ptr::eq(*field, from) && !std::ptr::eq(*field, to))
			.collect();
		let properties_name = format_ident!("{}Properties", name);
		let (properties_type, properties_struct) = if properties.is_empty() {
			(quote! { () }, quote! {})
		} else {
			let declarations = properties.iter().map(|(_, declaration)| declaration);
			let doc = format!(
				" What a new [`{name}`] holds besides its two ends, for `add`: every column the \
				 database layer doesn't fill in."
			);
			(
				quote! { #properties_name },
				quote! {
					#[doc = #doc]
					#vis struct #properties_name {
						#(#declarations,)*
					}
				},
			)
		};
		let assignments = properties.iter().map(|(field, _)| {
			let ident = &field.ident;
			quote! { #ident: properties.#ident }
		});
		let ignore = properties
			.is_empty()
			.then(|| quote! { let () = properties; });
		let writable = self.args.pack.is_none().then(|| {
			quote! { impl #krate::WritableLink for #name {} }
		});
		let end = |method: Option<&Ident>, near: &syn::Path, far: &syn::Path, forward: bool| {
			let method = method?;
			let via = format_ident!("__via_{}", unraw(method), span = method.span());
			let (constructor, near_column, far_column) = if forward {
				(quote!(forward), from_column, to_column)
			} else {
				(quote!(backward), to_column, from_column)
			};
			let doc = format!(
				" The `{}` rows linked to this one through [`{name}`].",
				quote!(#far)
			);
			Some(quote_spanned! { method.span()=>
				impl #near {
					#[doc = #doc]
					pub fn #method(&self) -> #krate::Links<#name, #near, #far> {
						#krate::Links::#constructor(<Self as #krate::HasId>::id(self))
					}

					/// The link, for shapes to follow.
					#[doc(hidden)]
					pub fn #via() -> #krate::__private::shape::Relation<Self, #far, #krate::__private::shape::Many> {
						#krate::__private::shape::Relation::through::<#name>(#near_column, #far_column)
					}
				}
			})
		};
		let forward = end(link.from.method.as_ref(), from_table, to_table, true);
		let backward = end(link.to.method.as_ref(), to_table, from_table, false);

		quote! {
			#properties_struct

			impl #krate::Link for #name {
				type From = #from_table;
				type To = #to_table;
				type Properties = #properties_type;
				const FROM: &'static str = #from_column;
				const TO: &'static str = #to_column;

				fn link(
					from: #krate::Id<#from_table>,
					to: #krate::Id<#to_table>,
					properties: Self::Properties,
				) -> Self {
					#ignore
					#new { #from_ident: from, #to_ident: to, #(#assignments),* }.into_row()
				}

				fn source_id(&self) -> #krate::Id<#from_table> {
					self.#from_ident
				}

				fn target_id(&self) -> #krate::Id<#to_table> {
					self.#to_ident
				}
			}

			#writable
			#forward
			#backward
		}
	}

	/// The table's public columns only, so a public shape that names any
	/// other fails to compile.
	fn public_columns(&self) -> TokenStream {
		let krate = self.krate;
		let name = self.name;
		let vis = &self.item.vis;
		let public_columns = format_ident!("{}PublicColumns", name);
		let columns: Vec<Column<'_>> = self
			.columns()
			.into_iter()
			.filter(|column| {
				column.field.args.visibility == Visibility::Public
					&& column.args.visibility == Visibility::Public
			})
			.collect();
		let fields = columns.iter().map(|column| {
			let handle = column.handle;
			let ty = &column.ty;
			quote! { pub #handle: #krate::Col<#name, #ty> }
		});
		let handles = columns.iter().map(|column| {
			let handle = column.handle;
			let name = column.name;
			quote! { #handle: #krate::Col::new(#name) }
		});

		quote! {
			/// The columns a public shape may read.
			#[doc(hidden)]
			#vis struct #public_columns {
				#(#fields,)*
			}

			impl #name {
				/// The columns a public shape may read.
				#[doc(hidden)]
				pub fn __public_columns() -> &'static #public_columns {
					static COLUMNS: #public_columns = #public_columns { #(#handles),* };
					&COLUMNS
				}
			}
		}
	}

	/// What an insert takes: every column except the generated key, the
	/// timestamps, and the version, which the ORM fills in. A flattened mixin
	/// is provided whole when the insert provides any of its columns.
	fn new_type(&self) -> TokenStream {
		let krate = self.krate;
		let name = self.name;
		let new = self.new_name();
		let vis = &self.item.vis;
		let columns = self.columns();
		let declarations = self
			.provided_fields()
			.into_iter()
			.map(|(_, declaration)| declaration);
		let assignments = self.fields.iter().map(|field| {
			let ident = &field.ident;
			let span = located(field.ty.span());

			match &field.mixin {
				None if is_provided(field) => quote! { #ident: self.#ident },
				None if field.generated => quote_spanned! { span=> #ident: #krate::Id::new() },
				None => {
					let value = automatic(&field.args);
					quote_spanned! { span=> #ident: #value }
				}
				Some(mixin)
					if mixin
						.columns
						.iter()
						.all(|inner| is_provided_args(&inner.args)) =>
				{
					quote! { #ident: self.#ident }
				}
				Some(mixin) => {
					let path = &mixin.path;
					let inner = mixin.columns.iter().map(|inner| {
						let inner_ident = &inner.ident;

						if is_provided_args(&inner.args) {
							quote! { #inner_ident: self.#ident.#inner_ident }
						} else {
							let value = automatic(&inner.args);
							quote_spanned! { span=> #inner_ident: #value }
						}
					});
					quote! { #ident: #path { #(#inner),* } }
				}
			}
		});
		let validate = validations(krate, columns.iter().filter(|column| column.is_provided()));
		let doc = format!(
			" A new [`{name}`] to insert: everything except what the database layer fills in."
		);

		quote! {
			#[doc = #doc]
			#vis struct #new {
				#(#declarations,)*
			}

			impl #new {
				/// The complete row, with its key and timestamps filled in.
				pub fn into_row(self) -> #name {
					#[allow(unused_variables, reason = "used only when the table has timestamps")]
					let now = #krate::now();
					#name { #(#assignments),* }
				}

				/// Checks every rule the columns declare, before anything reaches the database.
				pub fn validate(&self) -> ::core::result::Result<(), #krate::ValidationErrors> {
					#validate
				}
			}
		}
	}

	fn public_name(&self) -> Option<Ident> {
		match &self.args.public {
			Some(PublicName::Named(name)) => Some(name.clone()),
			Some(PublicName::Default) => Some(format_ident!("Public{}", self.name)),
			None => None,
		}
	}

	/// The client's view of a row: every column that isn't `private` or
	/// `secret`, with each flattened mixin as its own public type. Always
	/// compiled, so the client and generated RPC clients share it.
	fn public_type(&self) -> TokenStream {
		let Some(public) = self.public_name() else {
			return quote! {};
		};
		let name = self.name;
		let doc = format!(
			" What a client may see of a [`{name}`]: every column except the `private` and \
			 `secret` ones. Return this from server functions."
		);

		self.client_type(&public, None, &doc)
	}

	/// One type per `projection(...)`: the public columns, plus the
	/// `private` ones marked `in(...)` it.
	fn projections(&self) -> Vec<TokenStream> {
		let name = self.name;

		self.args
			.projections
			.iter()
			.map(|projection| {
				let doc = format!(
					" A [`{name}`] as `{projection}` may see it: every public column, and the \
					 private ones marked `in({projection})`."
				);
				self.client_type(projection, Some(projection), &doc)
			})
			.collect()
	}

	/// A serializable view of a row for an audience: the public type when
	/// `audience` is `None`, or else a projection.
	fn client_type(&self, type_name: &Ident, audience: Option<&Ident>, doc: &str) -> TokenStream {
		let krate = self.krate;
		let name = self.name;
		let vis = &self.item.vis;
		let serde_path = format!("{krate}::__private::serde").replace(' ', "");
		let schemars_path = format!("{krate}::__private::schemars").replace(' ', "");
		let sees = |args: &FieldArgs| {
			args.visibility == Visibility::Public
				|| (args.visibility == Visibility::Private
					&& audience.is_some_and(|audience| args.projections.contains(audience)))
		};
		let visible: Vec<&Field> = self
			.fields
			.iter()
			.filter(|field| sees(&field.args))
			.filter(|field| {
				field.mixin.as_ref().is_none_or(|mixin| {
					mixin
						.columns
						.iter()
						.any(|inner| inner.args.visibility == Visibility::Public)
				})
			})
			.collect();
		let declarations = visible.iter().map(|field| {
			let Field { ident, ty, .. } = field;
			let docs = field
				.attrs
				.iter()
				.filter(|attr| attr.path().is_ident("doc"));

			match &field.mixin {
				None => {
					let schema = json_schema_rules(&field.args);
					quote! { #(#docs)* #schema pub #ident: #ty }
				}
				Some(mixin) => {
					let path = &mixin.path;
					quote! { #(#docs)* pub #ident: <#path as #krate::Mixin>::Public }
				}
			}
		});
		let conversions = visible.iter().map(|field| {
			let ident = &field.ident;

			if field.mixin.is_some() {
				quote! { #ident: ::core::convert::From::from(row.#ident) }
			} else {
				quote! { #ident: row.#ident }
			}
		});
		let columns = self.columns();
		let validate = validations(
			krate,
			columns.iter().filter(|column| {
				sees(&column.field.args)
					&& (column.inner.is_none() || column.args.visibility == Visibility::Public)
			}),
		);

		quote! {
			#[doc = #doc]
			#[derive(
				::core::clone::Clone,
				::core::fmt::Debug,
				::core::cmp::PartialEq,
				#krate::__private::serde::Serialize,
				#krate::__private::serde::Deserialize,
				#krate::__private::schemars::JsonSchema,
			)]
			#[serde(crate = #serde_path)]
			#[schemars(crate = #schemars_path)]
			#vis struct #type_name {
				#(#declarations,)*
			}

			impl #type_name {
				/// Checks every rule the visible columns declare, such as in a form
				/// before it is sent.
				pub fn validate(&self) -> ::core::result::Result<(), #krate::ValidationErrors> {
					#validate
				}
			}

			#krate::__server! {
				impl ::core::convert::From<#name> for #type_name {
					fn from(row: #name) -> Self {
						Self { #(#conversions),* }
					}
				}
			}
		}
	}
}

/// Whether an insert provides the field, rather than the ORM.
fn is_provided(field: &Field) -> bool {
	!field.generated && is_provided_args(&field.args)
}

/// Whether an insert provides a column with these options, rather than the
/// ORM.
pub(crate) fn is_provided_args(args: &FieldArgs) -> bool {
	args.timestamp.is_none() && !args.version && args.generated.is_none()
}

/// The value the ORM gives a column on insert.
fn automatic(args: &FieldArgs) -> TokenStream {
	if args.generated.is_some() {
		// A placeholder: the database computes the value, and every write
		// returns the row as stored.
		quote! { ::core::default::Default::default() }
	} else if args.version {
		quote! { 0 }
	} else if args.timestamp == Some(Timestamp::Deleted) {
		quote! { ::core::option::Option::None }
	} else {
		quote! { now }
	}
}

fn validations<'a>(
	krate: &TokenStream,
	columns: impl Iterator<Item = &'a Column<'a>>,
) -> TokenStream {
	let mut checks = Vec::new();

	for column in columns {
		let access = &column.access;
		let label = column.label();
		let args = column.args;
		let span = column.span;

		if args.min_len.is_some() || args.max_len.is_some() {
			let min = option_u32(args.min_len.as_ref());
			let max = option_u32(args.max_len.as_ref());
			checks.push(quote_spanned! { span=>
				#krate::__private::rules::length(&mut errors, #label, &self.#access, #min, #max);
			});
		}

		if args.min.is_some() || args.max.is_some() {
			let min = option_f64(args.min.as_ref());
			let max = option_f64(args.max.as_ref());
			checks.push(quote_spanned! { span=>
				#krate::__private::rules::range(&mut errors, #label, &self.#access, #min, #max);
			});
		}

		if !args.one_of.is_empty() {
			let values = &args.one_of;
			checks.push(quote_spanned! { span=>
				#krate::__private::rules::one_of(&mut errors, #label, &self.#access, &[#(#values),*]);
			});
		}

		if let Some(pattern) = &args.regex {
			checks.push(quote_spanned! { span=>
				#krate::__private::rules::pattern(&mut errors, #label, &self.#access, #pattern);
			});
		}
	}

	if checks.is_empty() {
		return quote! { ::core::result::Result::Ok(()) };
	}

	quote! {
		let mut errors = #krate::ValidationErrors::default();
		#(#checks)*
		errors.into_result()
	}
}

/// The JSON Schema rules for a public field, so generated clients know them.
pub(crate) fn json_schema_rules(args: &FieldArgs) -> TokenStream {
	let mut rules = Vec::new();

	if args.min_len.is_some() || args.max_len.is_some() {
		let min = args.min_len.as_ref().map(|min| quote!(min = #min));
		let max = args.max_len.as_ref().map(|max| quote!(max = #max));
		let parts = min.into_iter().chain(max);
		rules.push(quote! { #[schemars(length(#(#parts),*))] });
	}

	if args.min.is_some() || args.max.is_some() {
		let min = args.min.as_ref().map(|min| quote!(min = #min));
		let max = args.max.as_ref().map(|max| quote!(max = #max));
		let parts = min.into_iter().chain(max);
		rules.push(quote! { #[schemars(range(#(#parts),*))] });
	}

	if let Some(pattern) = &args.regex {
		rules.push(quote! { #[schemars(regex(pattern = #pattern))] });
	}

	quote! { #(#rules)* }
}

fn default_value(krate: &TokenStream, default: &DefaultSpec) -> TokenStream {
	let value = quote!(#krate::__private::schema::DefaultValue);

	match default {
		DefaultSpec::Now => quote! { #value::Now },
		DefaultSpec::Sql(sql) => quote! { #value::Sql(::std::string::String::from(#sql)) },
		DefaultSpec::Negative(lit @ syn::Lit::Float(_)) => quote! { #value::Float(-#lit) },
		DefaultSpec::Negative(lit) => quote! { #value::Integer(-#lit) },
		DefaultSpec::Lit(lit) => {
			match lit {
				syn::Lit::Str(text) => quote! { #value::Text(::std::string::String::from(#text)) },
				syn::Lit::Bool(flag) => quote! { #value::Bool(#flag) },
				syn::Lit::Float(_) => quote! { #value::Float(#lit) },
				_ => quote! { #value::Integer(#lit) },
			}
		}
	}
}

/// `Some(expression)` or `None`, as an expression.
fn option_tokens(value: Option<TokenStream>) -> TokenStream {
	value.map_or_else(
		|| quote! { ::core::option::Option::None },
		|value| quote! { ::core::option::Option::Some(#value) },
	)
}

fn option_str(value: Option<String>) -> TokenStream {
	option_tokens(value.map(|value| quote!(#value)))
}

fn option_u32(value: Option<&syn::Expr>) -> TokenStream {
	option_tokens(value.map(|value| quote!((#value) as u32)))
}

fn option_f64(value: Option<&syn::Expr>) -> TokenStream {
	option_tokens(value.map(|value| quote!((#value) as f64)))
}

/// The `T` in `Id<T>` or `Option<Id<T>>`.
fn id_target(ty: &Type) -> Option<&Type> {
	let inner = generic_argument(ty, "Option").unwrap_or(ty);
	generic_argument(inner, "Id")
}

/// The single type argument of `ty` when its last segment is `wrapper`.
fn generic_argument<'a>(ty: &'a Type, wrapper: &str) -> Option<&'a Type> {
	let Type::Path(path) = ty else {
		return None;
	};
	let segment = path.path.segments.last()?;

	if segment.ident != wrapper {
		return None;
	}

	let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
		return None;
	};

	match arguments.args.first()? {
		GenericArgument::Type(inner) => Some(inner),
		_ => None,
	}
}

fn is_type_named(ty: &Type, name: &Ident) -> bool {
	matches!(ty, Type::Path(path) if path.qself.is_none() && path.path.is_ident(name))
}

pub(crate) fn unraw(ident: &Ident) -> String {
	let text = ident.to_string();
	text.strip_prefix("r#")
		.map_or(text.clone(), ToString::to_string)
}

fn combine(errors: &mut Option<syn::Error>, error: syn::Error) {
	match errors {
		Some(errors) => errors.combine(error),
		None => *errors = Some(error),
	}
}

/// Replaces `Self` in field types with the table's name: the same types also
/// appear in the generated insert and public types, where `Self` would mean
/// something else.
struct ReplaceSelf<'a>(&'a Ident);

impl VisitMut for ReplaceSelf<'_> {
	fn visit_type_path_mut(&mut self, path: &mut syn::TypePath) {
		if path.qself.is_none() && path.path.is_ident("Self") {
			path.path = self.0.clone().into();
		}

		syn::visit_mut::visit_type_path_mut(self, path);
	}
}

/// A span that points at `span` in errors but resolves names where the
/// macro was called. Generated code refers to its own locals (`table`,
/// `values`, `self`), which must resolve the same way however the expansion
/// was reached: through a mixin's macro, whose expansion has its own
/// hygiene, as much as directly.
fn located(span: Span) -> Span {
	span.resolved_at(Span::call_site())
}

/// The accessor a link gets: its field without the `_id` suffix, so
/// `author_id` gives `author()`.
fn link_name(field: &Ident) -> Ident {
	let name = unraw(field);
	let link = name
		.strip_suffix("_id")
		.filter(|link| !link.is_empty())
		.unwrap_or(&name);
	format_ident!("{link}", span = field.span())
}

/// What a `#[db::policy]` expands to when it runs at all: it sits above
/// `#[db::table]`, or on something that isn't a table, so nothing reads it.
/// The item is kept, so the error is the only one.
pub(crate) fn misplaced_policy(item: TokenStream) -> TokenStream {
	let mut tokens = syn::Error::new(
		Span::call_site(),
		"put `#[db::policy]` below `#[db::table]` or `#[db::link]`, which reads it",
	)
	.to_compile_error();
	tokens.extend(item);
	tokens
}
