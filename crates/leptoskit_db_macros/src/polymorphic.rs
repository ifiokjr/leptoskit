//! Expands `#[db::polymorphic]`: an enum whose variants are tables, stored
//! with class-table inheritance in a supertype table of keys and kinds.

use convert_case::Case;
use convert_case::Converter;
use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use quote::quote_spanned;
use syn::Attribute;
use syn::Fields;
use syn::Ident;
use syn::ItemEnum;
use syn::LitStr;
use syn::Path;
use syn::Type;
use syn::spanned::Spanned;

use crate::attrs::is_db_attr;
use crate::attrs::is_policy_attr;

/// What `#[db::polymorphic(...)]` takes.
struct PolymorphicArgs {
	name: LitStr,
	namespace: Option<LitStr>,
	public: bool,
	live: bool,
	audit: bool,
	krate: Option<Path>,
}

impl PolymorphicArgs {
	fn parse(tokens: TokenStream) -> syn::Result<Self> {
		let mut name = None;
		let mut namespace = None;
		let mut public = false;
		let mut live = false;
		let mut audit = false;
		let mut krate = None;

		let parser = syn::meta::parser(|meta| {
			if meta.path.is_ident("name") {
				name = Some(meta.value()?.parse::<LitStr>()?);
			} else if meta.path.is_ident("namespace") {
				namespace = Some(meta.value()?.parse::<LitStr>()?);
			} else if meta.path.is_ident("public") {
				public = true;
			} else if meta.path.is_ident("live") {
				live = true;
			} else if meta.path.is_ident("audit") {
				audit = true;
			} else if meta.path.is_ident("crate") {
				krate = Some(meta.value()?.parse::<LitStr>()?.parse()?);
			} else {
				return Err(meta.error(
					"unknown polymorphic option; expected `name`, `namespace`, `public`, `live`, \
					 `audit`, or `crate`",
				));
			}

			Ok(())
		});
		syn::parse::Parser::parse2(parser, tokens)?;

		let name = name.ok_or_else(|| {
			syn::Error::new(
				Span::call_site(),
				"a polymorphic table needs an explicit name for its supertype table, such as \
				 #[db::polymorphic(name = \"actors\")]",
			)
		})?;

		Ok(Self {
			name,
			namespace,
			public,
			live,
			audit,
			krate,
		})
	}
}

/// One variant: a table, and the kind its supertype rows record.
struct Variant<'a> {
	ident: &'a Ident,
	ty: &'a Type,
	/// The kind its supertype rows record: its name in `snake_case`, or
	/// its `#[db(rename = "…")]`.
	kind: String,
	/// Its other attributes, such as its docs.
	attrs: Vec<&'a Attribute>,
}

pub(crate) fn expand(args: TokenStream, mut item: ItemEnum) -> syn::Result<TokenStream> {
	let args = PolymorphicArgs::parse(args)?;
	let krate = crate::runtime(args.krate.as_ref());

	if !item.generics.params.is_empty() {
		return Err(syn::Error::new_spanned(
			&item.generics,
			"a polymorphic table can't be generic",
		));
	}

	// The supertype's policies go to its table.
	let mut policies = Vec::new();
	item.attrs.retain(|attr| {
		if is_policy_attr(attr) {
			policies.push(attr.clone());
			false
		} else {
			true
		}
	});

	if let Some(attr) = item.attrs.iter().find(|attr| is_db_attr(attr)) {
		return Err(syn::Error::new_spanned(
			attr,
			"a polymorphic table takes its options in `#[db::polymorphic(...)]`",
		));
	}

	let mut variants = Vec::new();

	for variant in &item.variants {
		let Fields::Unnamed(fields) = &variant.fields else {
			return Err(syn::Error::new_spanned(
				variant,
				"each variant holds one table, such as `User(User)`",
			));
		};
		let [field] = fields.unnamed.iter().collect::<Vec<_>>()[..] else {
			return Err(syn::Error::new_spanned(
				variant,
				"each variant holds one table, such as `User(User)`",
			));
		};

		if variant.discriminant.is_some() {
			return Err(syn::Error::new_spanned(
				variant,
				"a polymorphic table's variants have no discriminants",
			));
		}

		let mut kind = None;

		for attr in variant.attrs.iter().filter(|attr| is_db_attr(attr)) {
			attr.parse_nested_meta(|meta| {
				if meta.path.is_ident("rename") {
					kind = Some(meta.value()?.parse::<LitStr>()?.value());
					Ok(())
				} else {
					Err(meta.error(
						"a variant takes `rename = \"...\"`, the kind its supertype rows record",
					))
				}
			})?;
		}

		variants.push(Variant {
			ident: &variant.ident,
			ty: &field.ty,
			kind: kind.unwrap_or_else(|| {
				Converter::new()
					.from_case(Case::Pascal)
					.to_case(Case::Snake)
					.convert(variant.ident.to_string())
			}),
			attrs: variant
				.attrs
				.iter()
				.filter(|attr| !is_db_attr(attr))
				.collect(),
		});
	}

	if variants.is_empty() {
		return Err(syn::Error::new_spanned(
			&item.ident,
			"a polymorphic table needs at least one variant",
		));
	}

	for (index, variant) in variants.iter().enumerate() {
		if let Some(other) = variants[..index]
			.iter()
			.find(|other| type_text(other.ty) == type_text(variant.ty))
		{
			return Err(syn::Error::new_spanned(
				variant.ty,
				format!(
					"`{}` is already the `{}` variant: each table is one variant",
					type_text(variant.ty),
					other.ident
				),
			));
		}
	}

	Ok(Polymorphic {
		krate: &krate,
		args: &args,
		item: &item,
		variants: &variants,
		policies: &policies,
	}
	.generate())
}

struct Polymorphic<'a> {
	krate: &'a TokenStream,
	args: &'a PolymorphicArgs,
	item: &'a ItemEnum,
	variants: &'a [Variant<'a>],
	policies: &'a [Attribute],
}

impl Polymorphic<'_> {
	fn generate(&self) -> TokenStream {
		let krate = self.krate;
		let name = &self.item.ident;
		let vis = &self.item.vis;
		let kind = format_ident!("{name}Kind");
		let row = format_ident!("{name}Row");
		let docs: Vec<&Attribute> = self
			.item
			.attrs
			.iter()
			.filter(|attr| attr.path().is_ident("doc"))
			.collect();
		let attrs = &self.item.attrs;
		let idents: Vec<&Ident> = self.variants.iter().map(|variant| variant.ident).collect();
		let types: Vec<&Type> = self.variants.iter().map(|variant| variant.ty).collect();
		let variant_attrs = self.variants.iter().map(|variant| &variant.attrs);
		let kinds = self.variants.iter().map(|variant| &variant.kind);
		let kind_doc =
			format!(" Which kind of [`{name}`] a row is, as its supertype row stores it.");
		let public = self.public_type(&kind);
		let server = self.server_items(&kind, &row);
		let policy_import = self.policies.iter().find_map(|attr| {
			(attr.path().segments.len() == 1).then(|| {
				let path = attr.path();
				quote! {
					const _: () = {
						#[allow(unused_imports)]
						use #path as _;
					};
				}
			})
		});

		quote! {
			#[doc = #kind_doc]
			#[derive(
				::core::clone::Clone,
				::core::marker::Copy,
				::core::fmt::Debug,
				::core::cmp::PartialEq,
				::core::cmp::Eq,
				::core::hash::Hash,
				::core::cmp::PartialOrd,
				::core::cmp::Ord,
				#krate::Enum,
			)]
			#vis enum #kind {
				#(#[db(rename = #kinds)] #idents,)*
			}

			#public

			#krate::__server! {
				#(#attrs)*
				#vis enum #name {
					#(#(#variant_attrs)* #idents(#types),)*
				}

				#server
			}

			#krate::__client! {
				#(#docs)*
				///
				/// On the client this is only a marker: it names the table in
				/// `Id<…>`, and has no rows to construct.
				#vis enum #name {}
			}

			#policy_import
		}
	}

	/// The supertype table, and every trait and method that makes the enum
	/// a table to read, link to, and write through.
	fn server_items(&self, kind: &Ident, row: &Ident) -> TokenStream {
		let name = &self.item.ident;
		let idents: Vec<&Ident> = self.variants.iter().map(|variant| variant.ident).collect();
		let row_table = self.row_table(kind, row);
		let variants = self.variant_impls(kind);
		let polymorphic = self.polymorphic_impl(kind, row);
		let links = self.link_impls(row);
		let methods = self.methods(kind, row);
		let from_public = self.args.public.then(|| {
			let public = format_ident!("Public{name}");
			quote! {
				impl ::core::convert::From<#name> for #public {
					fn from(row: #name) -> Self {
						match row {
							#(#name::#idents(row) => Self::#idents(::core::convert::From::from(row)),)*
						}
					}
				}
			}
		});

		quote! {
			#row_table
			#variants
			#polymorphic
			#links
			#methods
			#from_public
		}
	}

	/// `{Enum}Row`: the supertype table, keyed by `Id<{Enum}>`, which the
	/// supertype's policies govern.
	fn row_table(&self, kind: &Ident, row: &Ident) -> TokenStream {
		let krate = self.krate;
		let name = &self.item.ident;
		let vis = &self.item.vis;
		let table_name = &self.args.name;
		let namespace = self
			.args
			.namespace
			.as_ref()
			.map(|namespace| quote!(namespace = #namespace,));
		let krate_option = self.args.krate.as_ref().map(|path| {
			let path = quote!(#path).to_string().replace(' ', "");
			quote!(crate = #path,)
		});
		let live = self.args.live.then(|| quote!(live,));
		let audit = self.args.audit.then(|| quote!(audit,));
		let policies = self.policies;
		let row_doc = format!(
			" The `{}` table: one row per `{name}`, of any kind, holding its key and its kind. \
			 Each variant's inserts and removals write it; don't write it yourself.",
			self.args.name.value()
		);

		quote! {
			#[#krate::table(name = #table_name, #namespace #krate_option #live #audit)]
			#(#policies)*
			#[doc = #row_doc]
			#vis struct #row {
				/// The key the variant row shares.
				#[db(key, generate = false, no_foreign_key)]
				pub id: #krate::Id<#name>,
				/// Which variant table holds the row.
				pub kind: #kind,
			}
		}
	}

	/// For each variant: `From` it, `Variant`, and a check that its table
	/// says it extends the enum. Also the enum's `Debug`.
	fn variant_impls(&self, kind: &Ident) -> TokenStream {
		let krate = self.krate;
		let name = &self.item.ident;
		let idents: Vec<&Ident> = self.variants.iter().map(|variant| variant.ident).collect();
		let types: Vec<&Type> = self.variants.iter().map(|variant| variant.ty).collect();
		let checks = self.variants.iter().map(|variant| {
			let ty = variant.ty;
			let message = format!(
				"`{}` is a variant of `{name}`, a polymorphic table: declare it with \
				 `#[db::table(extends = {name}, …)]`",
				type_text(ty)
			);
			quote_spanned! { ty.span()=>
				const _: () = ::core::assert!(
					<#ty as #krate::Table>::SUPERTYPE.is_some(),
					#message,
				);
			}
		});

		quote! {
			impl ::core::fmt::Debug for #name {
				fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
					match self {
						#(Self::#idents(row) => formatter.debug_tuple(::core::stringify!(#idents)).field(row).finish(),)*
					}
				}
			}

			#(
				impl ::core::convert::From<#types> for #name {
					fn from(row: #types) -> Self {
						Self::#idents(row)
					}
				}

				impl #krate::Variant for #types {
					type Of = #name;

					const KIND: #kind = #kind::#idents;

					fn into_supertype(self) -> #name {
						#name::#idents(self)
					}
				}
			)*

			#(#checks)*
		}
	}

	/// `Polymorphic`: the enum's kinds, and how each variant's rows load.
	fn polymorphic_impl(&self, kind: &Ident, row: &Ident) -> TokenStream {
		let krate = self.krate;
		let name = &self.item.ident;
		let idents: Vec<&Ident> = self.variants.iter().map(|variant| variant.ident).collect();
		let types: Vec<&Type> = self.variants.iter().map(|variant| variant.ty).collect();
		let loaders = self.loaders(kind);

		quote! {
			impl #krate::Polymorphic for #name {
				type Row = #row;
				type Kind = #kind;

				const KINDS: &'static [#kind] = &[#(#kind::#idents),*];

				fn id(&self) -> #krate::Id<Self> {
					match self {
						#(Self::#idents(row) => #krate::Id::upcast(<#types as #krate::HasId>::id(row)),)*
					}
				}

				fn kind(&self) -> #kind {
					match self {
						#(Self::#idents(_) => #kind::#idents,)*
					}
				}

				fn table_of(kind: #kind) -> #krate::__private::schema::TableName {
					match kind {
						#(#kind::#idents => <#types as #krate::Table>::table_name(),)*
					}
				}

				fn row(id: #krate::Id<Self>, kind: #kind) -> #row {
					#row { id, kind }
				}

				fn row_kind(row: &#row) -> #kind {
					row.kind
				}

				#loaders
			}
		}
	}

	/// `Polymorphic::fetch` and `Polymorphic::count`: one variant's rows
	/// on a page, by kind.
	fn loaders(&self, kind: &Ident) -> TokenStream {
		let krate = self.krate;
		let idents: Vec<&Ident> = self.variants.iter().map(|variant| variant.ident).collect();
		let types: Vec<&Type> = self.variants.iter().map(|variant| variant.ty).collect();
		let fetched = self.future(&quote!(::std::vec::Vec<Self>));
		let counted = self.future(&quote!(u64));

		quote! {
				fn fetch(
					connection: &impl #krate::Connection,
					kind: #kind,
					page: &#krate::__private::polymorphic::Page,
				) -> #fetched {
					async move {
						match kind {
							#(#kind::#idents => #krate::__private::polymorphic::fetch_variant::<#types>(connection, page).await,)*
						}
					}
				}

				fn count(
					connection: &impl #krate::Connection,
					kind: #kind,
					page: &#krate::__private::polymorphic::Page,
				) -> #counted {
					async move {
						match kind {
							#(#kind::#idents => #krate::__private::polymorphic::count_variant::<#types>(connection, page).await,)*
						}
					}
				}
		}
	}

	/// `Linkable` and `Referenced`: what makes `Id<{Enum}>` a foreign key
	/// to the supertype table and a link that loads any variant.
	fn link_impls(&self, row: &Ident) -> TokenStream {
		let krate = self.krate;
		let name = &self.item.ident;
		let loaded = self.future(&quote!(::core::option::Option<Self>));

		quote! {
			impl #krate::Linkable for #name {
				type Stored = #row;

				fn link_id(&self) -> #krate::Id<Self> {
					<Self as #krate::Polymorphic>::id(self)
				}

				fn load(connection: &impl #krate::Connection, id: #krate::Id<Self>) -> #loaded {
					#krate::__private::polymorphic::load(connection, id)
				}
			}

			impl #krate::Referenced for #name {
				fn referenced_table() -> #krate::__private::schema::TableName {
					<#row as #krate::Table>::table_name()
				}

				fn referenced_key() -> &'static [&'static str] {
					<#row as #krate::Table>::KEY
				}
			}
		}
	}

	/// The enum's own methods: a query, lookups by key, and writes that
	/// reach the variant row.
	fn methods(&self, kind: &Ident, row: &Ident) -> TokenStream {
		let krate = self.krate;
		let name = &self.item.ident;
		let idents: Vec<&Ident> = self.variants.iter().map(|variant| variant.ident).collect();

		quote! {
			impl #name {
				/// A query on every kind of row, or some kinds.
				pub fn query() -> #krate::Variants<Self> {
					#krate::Variants::new()
				}

				/// The row with this key, of whichever kind it is, if it exists,
				/// the request may see it, and it isn't soft-deleted.
				pub async fn find(
					connection: &impl #krate::Connection,
					id: #krate::Id<Self>,
				) -> #krate::Result<::core::option::Option<Self>> {
					#krate::__private::polymorphic::load(connection, id).await
				}

				/// The row with this key, or `DbError::NotFound`.
				pub async fn get(connection: &impl #krate::Connection, id: #krate::Id<Self>) -> #krate::Result<Self> {
					Self::find(connection, id).await?.ok_or_else(|| #krate::DbError::NotFound {
						table: <#row as #krate::Table>::flat_name(),
					})
				}

				/// The row's key, which its variant row and supertype row share.
				pub fn id(&self) -> #krate::Id<Self> {
					<Self as #krate::Polymorphic>::id(self)
				}

				/// Which kind of row it is.
				pub fn kind(&self) -> #kind {
					<Self as #krate::Polymorphic>::kind(self)
				}

				/// Writes every column of the variant row, as its table's `save`.
				pub async fn save(&mut self, connection: &impl #krate::Connection) -> #krate::Result<()> {
					match self {
						#(Self::#idents(row) => #krate::__private::write::save(connection, row).await,)*
					}
				}

				/// Deletes the variant row, as its table's `delete`: soft-deletes
				/// it when the table has a `#[db(deleted_at)]` column.
				pub async fn delete(&mut self, connection: &impl #krate::Connection) -> #krate::Result<()> {
					match self {
						#(Self::#idents(row) => #krate::__private::write::delete(connection, row).await,)*
					}
				}

				/// Removes the variant row and its supertype row for good.
				pub async fn destroy(self, connection: &impl #krate::Connection) -> #krate::Result<()> {
					match self {
						#(Self::#idents(row) => #krate::__private::write::destroy(connection, &row).await,)*
					}
				}
			}
		}
	}

	/// `impl Future<Output = Result<output>> + Send`.
	fn future(&self, output: &TokenStream) -> TokenStream {
		let krate = self.krate;
		quote! {
			impl ::core::future::Future<Output = #krate::Result<#output>> + ::core::marker::Send
		}
	}

	/// `Public{Enum}`: each variant's public type, tagged with its kind.
	fn public_type(&self, kind: &Ident) -> Option<TokenStream> {
		if !self.args.public {
			return None;
		}

		let krate = self.krate;
		let name = &self.item.ident;
		let vis = &self.item.vis;
		let public = format_ident!("Public{name}");
		let serde_path = format!("{krate}::__private::serde").replace(' ', "");
		let schemars_path = format!("{krate}::__private::schemars").replace(' ', "");
		let idents: Vec<&Ident> = self.variants.iter().map(|variant| variant.ident).collect();
		let publics = self.variants.iter().map(|variant| public_path(variant.ty));
		let renames = self.variants.iter().map(|variant| &variant.kind);
		let doc = format!(
			" What a client may see of a [`{name}`]: its variant's public type, tagged with its \
			 kind. Return this from server functions."
		);

		Some(quote! {
			#[doc = #doc]
			#[derive(
				::core::clone::Clone,
				::core::fmt::Debug,
				::core::cmp::PartialEq,
				#krate::__private::serde::Serialize,
				#krate::__private::serde::Deserialize,
				#krate::__private::schemars::JsonSchema,
			)]
			#[serde(crate = #serde_path, tag = "kind", content = "row")]
			#[schemars(crate = #schemars_path)]
			#vis enum #public {
				#(
					#[serde(rename = #renames)]
					#idents(#publics),
				)*
			}

			impl #public {
				/// Which kind of row it is.
				pub fn kind(&self) -> #kind {
					match self {
						#(Self::#idents(_) => #kind::#idents,)*
					}
				}
			}
		})
	}
}

/// The variant's public type: its path with `Public` before the last
/// segment, as `#[db::table(public)]` names it.
fn public_path(ty: &Type) -> TokenStream {
	match ty {
		Type::Path(path) if path.qself.is_none() => {
			let mut path = path.path.clone();

			if let Some(last) = path.segments.last_mut() {
				last.ident = format_ident!("Public{}", last.ident, span = last.ident.span());
			}

			quote!(#path)
		}
		other => {
			syn::Error::new_spanned(
				other,
				"a public polymorphic table's variants are named by path, such as `User`",
			)
			.to_compile_error()
		}
	}
}

fn type_text(ty: &Type) -> String {
	quote!(#ty).to_string().replace(' ', "")
}
