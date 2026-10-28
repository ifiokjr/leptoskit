//! Expands `#[db::table]` into the server row, its insert type, its client
//! projection, and its schema description.

use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use quote::quote_spanned;
use syn::Attribute;
use syn::Fields;
use syn::GenericArgument;
use syn::Ident;
use syn::ItemStruct;
use syn::PathArguments;
use syn::Type;
use syn::spanned::Spanned;
use syn::visit_mut::VisitMut;

use crate::attrs::DefaultSpec;
use crate::attrs::FieldArgs;
use crate::attrs::PublicName;
use crate::attrs::TableArgs;
use crate::attrs::Timestamp;
use crate::attrs::Visibility;
use crate::attrs::doc_comment;
use crate::attrs::is_db_attr;

/// Derives the generated code handles itself; deriving them as well would
/// either collide or leak secrets.
const RESERVED_DERIVES: &[(&str, &str)] = &[
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
}

pub(crate) fn expand(
	args: &TableArgs,
	mut item: ItemStruct,
	krate: &TokenStream,
) -> syn::Result<TokenStream> {
	if !item.generics.params.is_empty() {
		return Err(syn::Error::new_spanned(
			&item.generics,
			"a table can't be generic",
		));
	}

	check_derives(&item.attrs)?;

	let name = item.ident.clone();
	let Fields::Named(named) = &mut item.fields else {
		return Err(syn::Error::new_spanned(
			&item.fields,
			"a table needs named fields: one per column",
		));
	};

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
		});
	}

	if let Some(errors) = errors {
		return Err(errors);
	}

	resolve_keys(&name, &mut fields)?;
	check_markers(&name, &fields)?;

	let table = Table {
		krate,
		args,
		name: &name,
		item: &item,
		fields: &fields,
	};
	Ok(table.generate())
}

fn check_derives(attrs: &[Attribute]) -> syn::Result<()> {
	for attr in attrs.iter().filter(|attr| attr.path().is_ident("derive")) {
		attr.parse_nested_meta(|meta| {
			let derived = meta
				.path
				.segments
				.last()
				.map(|segment| segment.ident.to_string())
				.unwrap_or_default();

			if let Some((_, reason)) = RESERVED_DERIVES.iter().find(|(name, _)| *name == derived) {
				return Err(meta.error(format!("don't derive `{derived}` on a table: {reason}")));
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

fn check_markers(name: &Ident, fields: &[Field]) -> syn::Result<()> {
	for (marker, label) in [
		(Timestamp::Created, "created_at"),
		(Timestamp::Updated, "updated_at"),
		(Timestamp::Deleted, "deleted_at"),
	] {
		let marked: Vec<&Field> = fields
			.iter()
			.filter(|field| field.args.timestamp == Some(marker))
			.collect();

		if marked.len() > 1 {
			return Err(syn::Error::new_spanned(
				&marked[1].ident,
				format!("only one field can be #[db({label})]"),
			));
		}
	}

	for field in fields {
		if let Some(except) = &field.args.unique_except {
			if !fields.iter().any(|other| &other.ident == except) {
				return Err(syn::Error::new_spanned(
					except,
					format!("`{name}` has no field `{except}`"),
				));
			}
		}

		if field.key && field.args.timestamp.is_some() {
			return Err(syn::Error::new_spanned(
				&field.ident,
				"a key can't also be a timestamp",
			));
		}
	}

	Ok(())
}

struct Table<'a> {
	krate: &'a TokenStream,
	args: &'a TableArgs,
	name: &'a Ident,
	item: &'a ItemStruct,
	fields: &'a [Field],
}

impl Table<'_> {
	fn generate(&self) -> TokenStream {
		let krate = self.krate;
		let row = self.row();
		let debug = self.debug();
		let table_impl = self.table_impl();
		let new = self.new_type();
		let columns = self.columns_type();
		let methods = self.methods();
		let public = self.public_type();
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

	fn column_of(&self, ident: &Ident) -> String {
		self.fields
			.iter()
			.find(|field| &field.ident == ident)
			.map_or_else(|| ident.to_string(), |field| field.column.clone())
	}

	fn marked(&self, marker: Timestamp) -> Option<&Field> {
		self.fields
			.iter()
			.find(|field| field.args.timestamp == Some(marker))
	}

	/// The column carrying `marker`, as an `Option<&str>` expression.
	fn marker(&self, marker: Timestamp) -> TokenStream {
		option_str(self.marked(marker).map(|field| field.column.clone()))
	}

	fn table_impl(&self) -> TokenStream {
		let krate = self.krate;
		let name = self.name;
		let table_name = &self.args.name;
		let namespace = option_str(self.args.namespace.as_ref().map(syn::LitStr::value));
		let columns = self.fields.iter().map(|field| &field.column);
		let keys = self
			.fields
			.iter()
			.filter(|field| field.key)
			.map(|field| &field.column);
		let created = self.marker(Timestamp::Created);
		let updated = self.marker(Timestamp::Updated);
		let deleted = self.marker(Timestamp::Deleted);
		let definition = self.definition();
		let columns_name = self.columns_name();
		let column_handles = self.fields.iter().map(|field| {
			let ident = &field.ident;
			let column = &field.column;
			quote! { #ident: #krate::Col::new(#column) }
		});
		let validate = validations(krate, self.fields.iter());
		let to_values = self.fields.iter().map(|field| {
			let ident = &field.ident;
			quote_spanned! { field.ty.span()=> #krate::Column::to_value(&self.#ident)? }
		});
		let from_values = self.fields.iter().map(|field| {
			let ident = &field.ident;
			let column = &field.column;
			quote! { #ident: #krate::__private::decode_next(&mut values, #table_name, #column)? }
		});
		let touch = self.marked(Timestamp::Updated).map(|field| {
			let ident = &field.ident;
			quote_spanned! { field.ty.span()=> self.#ident = #krate::now(); }
		});
		let mark_deleted = self.marked(Timestamp::Deleted).map(|field| {
			let ident = &field.ident;
			quote_spanned! { field.ty.span()=> self.#ident = ::core::option::Option::Some(#krate::now()); }
		});

		quote! {
			impl #krate::Table for #name {
				const NAME: &'static str = #table_name;
				const NAMESPACE: ::core::option::Option<&'static str> = #namespace;
				const COLUMNS: &'static [&'static str] = &[#(#columns),*];
				const KEY: &'static [&'static str] = &[#(#keys),*];
				const CREATED_AT: ::core::option::Option<&'static str> = #created;
				const UPDATED_AT: ::core::option::Option<&'static str> = #updated;
				const DELETED_AT: ::core::option::Option<&'static str> = #deleted;
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
			}
		}
	}

	fn definition(&self) -> TokenStream {
		let krate = self.krate;
		let table_name = &self.args.name;
		let namespace = option_str(self.args.namespace.as_ref().map(syn::LitStr::value));
		let comment = option_str(doc_comment(&self.item.attrs));
		let renamed_from = option_str(self.args.renamed_from.as_ref().map(syn::LitStr::value));
		let mut calls: Vec<TokenStream> = self
			.fields
			.iter()
			.flat_map(|field| self.column_calls(field))
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
	fn column_calls(&self, field: &Field) -> Vec<TokenStream> {
		let krate = self.krate;
		let column = &field.column;
		let ty = &field.ty;
		let args = &field.args;
		let default = option_tokens(
			args.default
				.as_ref()
				.map(|default| default_value(krate, default)),
		);
		let comment = option_str(field.docs.clone());
		let renamed_from = option_str(args.renamed_from.as_ref().map(syn::LitStr::value));
		let mut calls = vec![quote_spanned! { ty.span()=>
			table.column::<#ty>(#column, #default, #comment, #renamed_from);
		}];

		if args.unique {
			let except = option_str(
				args.unique_except
					.as_ref()
					.map(|except| self.column_of(except)),
			);
			calls.push(quote! { table.unique(&[#column], #except); });
		}

		if args.index {
			calls.push(quote! { table.index(&[#column]); });
		}

		if args.min_len.is_some() || args.max_len.is_some() {
			let min = option_u32(args.min_len.as_ref());
			let max = option_u32(args.max_len.as_ref());
			calls.push(quote! { table.length(#column, #min, #max); });
		}

		if args.min.is_some() || args.max.is_some() {
			let min = option_f64(args.min.as_ref());
			let max = option_f64(args.max.as_ref());
			calls.push(quote! { table.range(#column, #min, #max); });
		}

		if !args.one_of.is_empty() {
			let values = &args.one_of;
			calls.push(quote! { table.one_of(#column, &[#(#values),*]); });
		}

		if let Some(pattern) = &args.regex {
			calls.push(quote! { table.pattern(#column, #pattern); });
		}

		// Only `Id<T>` columns reference another table, and a table's own
		// `Id<Self>` key references nothing.
		if id_target(ty).is_some() && !args.no_foreign_key && !(field.key && field.points_at_self) {
			let action = args.on_delete.as_ref().map(|action| {
				let variant = format_ident!(
					"{}",
					convert_case::Converter::new()
						.from_case(convert_case::Case::Snake)
						.to_case(convert_case::Case::Pascal)
						.convert(action.to_string())
				);
				quote! { #krate::__private::schema::ReferentialAction::#variant }
			});
			let action = option_tokens(action);
			calls.push(quote_spanned! { ty.span()=> table.foreign_key::<#ty>(#column, #action); });
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
		let fields = self.fields.iter().map(|field| {
			let Field { ident, ty, .. } = field;
			let docs = field
				.attrs
				.iter()
				.filter(|attr| attr.path().is_ident("doc"));
			quote! { #(#docs)* pub #ident: #krate::Col<#name, #ty> }
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
				pub fn update() -> #krate::Update<Self> {
					#krate::Update::new()
				}

				#by_key

				/// Writes every column, matched by the key, and bumps `updated_at`.
				pub async fn save(&mut self, connection: &impl #krate::Connection) -> #krate::Result<()> {
					#krate::__private::write::save(connection, self).await
				}

				/// Deletes the row: soft-deletes it when the table has a
				/// `#[db(deleted_at)]` column.
				pub async fn delete(&mut self, connection: &impl #krate::Connection) -> #krate::Result<()> {
					#krate::__private::write::delete(connection, self).await
				}

				/// Removes the row for good.
				pub async fn destroy(self, connection: &impl #krate::Connection) -> #krate::Result<()> {
					#krate::__private::write::destroy(connection, &self).await
				}
			}

			impl #new {
				/// Inserts the row and returns it as stored.
				pub async fn insert(self, connection: &impl #krate::Connection) -> #krate::Result<#name> {
					#krate::__private::write::insert(connection, self.into_row()).await
				}

				/// Inserts the row, or updates the existing row whose `target`
				/// columns match, keeping its key and `created_at`.
				pub async fn upsert<C: #krate::ConflictTarget>(
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

	fn new_name(&self) -> Ident {
		format_ident!("New{}", self.name)
	}

	/// What an insert takes: every column except the generated key and the
	/// timestamps the ORM fills in.
	fn new_type(&self) -> TokenStream {
		let krate = self.krate;
		let name = self.name;
		let new = self.new_name();
		let vis = &self.item.vis;
		let provided: Vec<&Field> = self
			.fields
			.iter()
			.filter(|field| is_provided(field))
			.collect();
		let declarations = provided.iter().map(|field| {
			let Field { ident, ty, vis, .. } = field;
			let docs = field
				.attrs
				.iter()
				.filter(|attr| attr.path().is_ident("doc"));
			quote! { #(#docs)* #vis #ident: #ty }
		});
		let assignments = self.fields.iter().map(|field| {
			let ident = &field.ident;

			if is_provided(field) {
				quote! { #ident: self.#ident }
			} else if field.generated {
				quote_spanned! { field.ty.span()=> #ident: #krate::Id::new() }
			} else if field.args.timestamp == Some(Timestamp::Deleted) {
				quote_spanned! { field.ty.span()=> #ident: ::core::option::Option::None }
			} else {
				quote_spanned! { field.ty.span()=> #ident: now }
			}
		});
		let validate = validations(krate, provided.iter().copied());
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
	/// `secret`. Always compiled, so the client and generated RPC clients
	/// share it.
	fn public_type(&self) -> TokenStream {
		let Some(public) = self.public_name() else {
			return quote! {};
		};
		let krate = self.krate;
		let name = self.name;
		let vis = &self.item.vis;
		let serde_path = format!("{krate}::__private::serde").replace(' ', "");
		let schemars_path = format!("{krate}::__private::schemars").replace(' ', "");
		let visible: Vec<&Field> = self
			.fields
			.iter()
			.filter(|field| field.args.visibility == Visibility::Public)
			.collect();
		let declarations = visible.iter().map(|field| {
			let Field { ident, ty, .. } = field;
			let docs = field
				.attrs
				.iter()
				.filter(|attr| attr.path().is_ident("doc"));
			let schema = json_schema_rules(field);
			quote! { #(#docs)* #schema pub #ident: #ty }
		});
		let conversions = visible.iter().map(|field| {
			let ident = &field.ident;
			quote! { #ident: row.#ident }
		});
		let validate = validations(krate, visible.iter().copied());
		let doc = format!(
			" What a client may see of a [`{name}`]: every column except the `private` and \
			 `secret` ones. Return this from server functions."
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
			#vis struct #public {
				#(#declarations,)*
			}

			impl #public {
				/// Checks every rule the visible columns declare, such as in a form
				/// before it is sent.
				pub fn validate(&self) -> ::core::result::Result<(), #krate::ValidationErrors> {
					#validate
				}
			}

			#krate::__server! {
				impl ::core::convert::From<#name> for #public {
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
	!field.generated && field.args.timestamp.is_none()
}

fn validations<'a>(krate: &TokenStream, fields: impl Iterator<Item = &'a Field>) -> TokenStream {
	let mut checks = Vec::new();

	for field in fields {
		let ident = &field.ident;
		let label = ident.to_string();
		let args = &field.args;

		if args.min_len.is_some() || args.max_len.is_some() {
			let min = option_u32(args.min_len.as_ref());
			let max = option_u32(args.max_len.as_ref());
			checks.push(quote_spanned! { field.ty.span()=>
				#krate::__private::rules::length(&mut errors, #label, &self.#ident, #min, #max);
			});
		}

		if args.min.is_some() || args.max.is_some() {
			let min = option_f64(args.min.as_ref());
			let max = option_f64(args.max.as_ref());
			checks.push(quote_spanned! { field.ty.span()=>
				#krate::__private::rules::range(&mut errors, #label, &self.#ident, #min, #max);
			});
		}

		if !args.one_of.is_empty() {
			let values = &args.one_of;
			checks.push(quote_spanned! { field.ty.span()=>
				#krate::__private::rules::one_of(&mut errors, #label, &self.#ident, &[#(#values),*]);
			});
		}

		if let Some(pattern) = &args.regex {
			checks.push(quote_spanned! { field.ty.span()=>
				#krate::__private::rules::pattern(&mut errors, #label, &self.#ident, #pattern);
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
fn json_schema_rules(field: &Field) -> TokenStream {
	let args = &field.args;
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

fn unraw(ident: &Ident) -> String {
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
