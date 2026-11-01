//! Expands `#[derive(db::Shape)]`: what to fetch about a row, and about the
//! rows linked to it, in one statement.
//!
//! The derive can't see the table's columns or links, so it names what the
//! table's macro generated and lets the compiler check it: each column field
//! reads `Table::columns().field` as a `Col<Table, FieldType>`, a public
//! shape reads `Table::__public_columns()` instead, which has no private
//! columns, and `via = posts` calls `Table::__via_posts()`, the relation the
//! link or backlink named `posts` declared.

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use quote::quote_spanned;
use syn::Data;
use syn::DeriveInput;
use syn::Fields;
use syn::Ident;
use syn::Path;
use syn::Type;
use syn::spanned::Spanned;

use crate::attrs::is_db_attr;

/// Both databases cap a function's arguments at 100, and each nested row is
/// one call with an argument per field.
const MAX_FIELDS: usize = 100;

/// The options on the shape: `#[db(from = User, public)]`.
struct ShapeArgs {
	from: Path,
	public: bool,
	krate: Option<Path>,
}

impl ShapeArgs {
	fn parse(input: &DeriveInput) -> syn::Result<Self> {
		let mut from = None;
		let mut public = false;
		let mut krate = None;

		for attr in input.attrs.iter().filter(|attr| is_db_attr(attr)) {
			attr.parse_nested_meta(|meta| {
				if meta.path.is_ident("from") {
					if from.is_some() {
						return Err(meta.error("a shape reads one table; `from` is given twice"));
					}

					from = Some(meta.value()?.parse()?);
				} else if meta.path.is_ident("public") {
					public = true;
				} else if meta.path.is_ident("crate") {
					krate = Some(meta.value()?.parse::<syn::LitStr>()?.parse()?);
				} else {
					return Err(meta.error(
						"unknown shape option; expected `from = Table`, `public`, or `crate`",
					));
				}

				Ok(())
			})?;
		}

		let from = from.ok_or_else(|| {
			syn::Error::new_spanned(
				&input.ident,
				"name the table the shape reads: #[db(from = User)]",
			)
		})?;

		Ok(Self {
			from,
			public,
			krate,
		})
	}
}

/// What one field reads.
#[derive(Clone)]
enum Source {
	/// A column of the table, by its handle.
	Column(Ident),
	/// How many rows a relation reaches.
	Count(Ident),
	/// The rows a relation reaches, as another shape.
	Via(Ident),
}

/// The options on one field.
struct FieldArgs {
	source: Option<Source>,
	/// Each must match: several narrow like several `Query::filter` calls.
	filter: Vec<syn::Expr>,
	order_by: Vec<syn::Expr>,
	limit: Option<syn::Expr>,
	offset: Option<syn::Expr>,
	/// Where `with_deleted` was given: soft-deleted linked rows count.
	with_deleted: Option<Span>,
}

impl FieldArgs {
	fn parse(field: &syn::Field) -> syn::Result<Self> {
		let mut args = Self {
			source: None,
			filter: Vec::new(),
			order_by: Vec::new(),
			limit: None,
			offset: None,
			with_deleted: None,
		};

		for attr in field.attrs.iter().filter(|attr| is_db_attr(attr)) {
			attr.parse_nested_meta(|meta| {
				let source = if meta.path.is_ident("column") {
					Some(Source::Column(meta.value()?.parse()?))
				} else if meta.path.is_ident("count") {
					let content;
					syn::parenthesized!(content in meta.input);
					Some(Source::Count(content.parse()?))
				} else if meta.path.is_ident("via") {
					Some(Source::Via(meta.value()?.parse()?))
				} else if meta.path.is_ident("filter") {
					args.filter.push(meta.value()?.parse()?);
					None
				} else if meta.path.is_ident("order_by") {
					args.order_by.push(meta.value()?.parse()?);
					None
				} else if meta.path.is_ident("limit") {
					once(&meta, &mut args.limit, "limit")?;
					None
				} else if meta.path.is_ident("offset") {
					once(&meta, &mut args.offset, "offset")?;
					None
				} else if meta.path.is_ident("with_deleted") {
					if args.with_deleted.is_some() {
						return Err(meta.error("`with_deleted` is given twice"));
					}

					args.with_deleted = Some(meta.path.span());
					None
				} else {
					return Err(meta.error(
						"unknown shape field option; expected `column = field`, `count(link)`, \
						 `via = link`, `filter = |row| …`, `order_by = |row| …`, `limit`, \
						 `offset`, or `with_deleted`",
					));
				};

				if let Some(source) = source {
					if args.source.is_some() {
						return Err(meta
							.error("a field reads one thing: a column, `count(…)`, or `via = …`"));
					}

					args.source = Some(source);
				}

				Ok(())
			})?;
		}

		Ok(args)
	}
}

/// Reads an option that may appear once.
fn once(
	meta: &syn::meta::ParseNestedMeta<'_>,
	slot: &mut Option<syn::Expr>,
	name: &str,
) -> syn::Result<()> {
	if slot.is_some() {
		return Err(meta.error(format!("`{name}` is given twice")));
	}

	*slot = Some(meta.value()?.parse()?);
	Ok(())
}

/// How many nested rows a `via` field holds, from its type.
enum Nesting<'a> {
	One(&'a Type),
	Optional(&'a Type),
	Many(&'a Type),
}

impl<'a> Nesting<'a> {
	fn of(ty: &'a Type) -> Self {
		if let Some(inner) = wrapped(ty, "Vec") {
			Self::Many(inner)
		} else if let Some(inner) = wrapped(ty, "Option") {
			Self::Optional(inner)
		} else {
			Self::One(ty)
		}
	}

	fn shape(&self) -> &'a Type {
		match self {
			Self::One(ty) | Self::Optional(ty) | Self::Many(ty) => ty,
		}
	}
}

pub(crate) fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
	let args = ShapeArgs::parse(input)?;
	let krate = crate::runtime(args.krate.as_ref());

	if !input.generics.params.is_empty() {
		return Err(syn::Error::new_spanned(
			&input.generics,
			"a shape can't be generic",
		));
	}

	let Data::Struct(data) = &input.data else {
		return Err(syn::Error::new_spanned(
			&input.ident,
			"a shape is a struct with a field for each value it reads",
		));
	};
	let Fields::Named(fields) = &data.fields else {
		return Err(syn::Error::new_spanned(
			&data.fields,
			"a shape needs named fields: one per value it reads",
		));
	};

	if fields.named.is_empty() {
		return Err(syn::Error::new_spanned(
			&input.ident,
			"a shape reads at least one value",
		));
	}

	if fields.named.len() > MAX_FIELDS {
		return Err(syn::Error::new_spanned(
			&input.ident,
			format!("a shape reads at most {MAX_FIELDS} values; nest some in another shape"),
		));
	}

	let name = &input.ident;
	let from = &args.from;
	let private = quote!(#krate::__private::shape);
	let shape = Shape {
		label: name.to_string(),
		from,
		private: &private,

		columns: if args.public {
			quote! { #from::__public_columns() }
		} else {
			quote! { <#from as #krate::Table>::columns() }
		},
	};
	let mut selection = Vec::new();

	let mut decoders = Vec::new();
	let mut public_nested = Vec::new();

	for field in &fields.named {
		let generated = shape.field(field)?;
		selection.push(generated.selected);
		decoders.push(generated.decoder);
		public_nested.extend(generated.nested.filter(|_| args.public));
	}

	let has_id = args
		.public
		.then(|| key_field(from, &fields.named))
		.flatten()
		.map(|key| {
			quote! {
				impl #krate::HasId<#from> for #name {
					fn id(&self) -> #krate::Id<#from> {
						self.#key
					}
				}
			}
		});
	let public = args.public.then(|| {
		quote! {
			impl #krate::PublicShape for #name {}

			const _: () = {
				fn public<S: #krate::PublicShape>() {}

				fn nested_shapes_are_public() {
					#(public::<#public_nested>();)*
				}
			};
		}
	});

	Ok(quote! {
		#krate::__server! {
			impl #krate::Shape for #name {
				type From = #from;

				fn selection() -> ::std::vec::Vec<#private::Selected> {
					::std::vec![#(#selection),*]
				}

				fn from_values(values: ::std::vec::Vec<#krate::Value>) -> #krate::Result<Self> {
					let mut values = values.into_iter();
					::core::result::Result::Ok(Self { #(#decoders),* })
				}
			}

			#public
		}

		#has_id
	})
}

/// The field holding the shaped row's key, when a shape selects it: its
/// table's `id` column, of type `Id<From>`. A client can then tell the rows
/// of a list apart by key.
fn key_field<'a>(
	from: &Path,
	fields: impl IntoIterator<Item = &'a syn::Field>,
) -> Option<&'a Ident> {
	fields.into_iter().find_map(|field| {
		let ident = field.ident.as_ref()?;
		let args = FieldArgs::parse(field).ok()?;
		let column = match &args.source {
			None => ident,
			Some(Source::Column(column)) => column,
			Some(_) => return None,
		};

		let target = wrapped(&field.ty, "Id")?;

		(column == "id" && quote!(#target).to_string() == quote!(#from).to_string())
			.then_some(ident)
	})
}

/// What the shape's fields expand against.
struct Shape<'a> {
	label: String,
	from: &'a Path,
	/// The runtime's shape module.
	private: &'a TokenStream,
	/// The table's column handles: only its public ones for a public shape.
	columns: TokenStream,
}

/// One field's part of the expansion.
struct Generated<'a> {
	/// Its entry in `Shape::selection`.
	selected: TokenStream,
	/// Its initializer in `Shape::from_values`.
	decoder: TokenStream,
	/// The shape it nests, if it nests one.
	nested: Option<&'a Type>,
}

impl Shape<'_> {
	fn field<'f>(&self, field: &'f syn::Field) -> syn::Result<Generated<'f>> {
		let Self {
			label,
			from,
			private,
			columns,
		} = self;
		let ident = field.ident.as_ref().expect("named fields have names");
		let ty = &field.ty;
		let field_label = crate::table::unraw(ident);
		let args = FieldArgs::parse(field)?;
		// Names in the generated code resolve where the derive is, wherever
		// errors point.
		let span = field.ty.span().resolved_at(Span::call_site());
		let refinements = refinements(&args);
		let source = args
			.source
			.as_ref()
			.map_or_else(|| Source::Column(ident.clone()), Source::clone);
		let decode_column = quote_spanned! { span=>
			#ident: #private::decode_column::<#ty>(&mut values, #label, #field_label)?
		};

		match source {
			Source::Column(column) => {
				if let Some(span) = refinements.first().map(Spanned::span).or(args.with_deleted) {
					return Err(syn::Error::new(
						span,
						"`filter`, `order_by`, `limit`, `offset`, and `with_deleted` choose \
						 linked rows; this field reads a column",
					));
				}

				Ok(Generated {
					selected: quote_spanned! { span=>
						#private::Selected::column::<#from, #ty>(#columns.#column)
					},
					decoder: decode_column,
					nested: None,
				})
			}
			Source::Count(link) => {
				if args.limit.is_some() || args.offset.is_some() || !args.order_by.is_empty() {
					return Err(syn::Error::new_spanned(
						&link,
						"a count takes `filter`, but not `order_by`, `limit`, or `offset`",
					));
				}

				let via = via(&link);
				let with_deleted = with_deleted(&args);
				Ok(Generated {
					selected: quote_spanned! { link.span()=>
						#private::Selected::from(#from::#via().count() #(#refinements)* #with_deleted)
					},
					decoder: decode_column,
					nested: None,
				})
			}
			Source::Via(link) => {
				let nesting = Nesting::of(ty);
				let shape = nesting.shape();
				let via = via(&link);
				let (fetch, decode) = match nesting {
					Nesting::One(_) => {
						if let Some(option) = refinements.first() {
							return Err(syn::Error::new(
								option.span(),
								"a required nested row is the one its link points at; to narrow, \
								 order, or skip rows, make the field an `Option` or a `Vec`",
							));
						}

						(quote!(one), quote!(decode_one))
					}
					Nesting::Optional(_) => {
						if let Some(limit) = &args.limit {
							return Err(syn::Error::new_spanned(
								limit,
								"an `Option` holds the first row; `limit` is for a `Vec`",
							));
						}

						(quote!(optional), quote!(decode_optional))
					}
					Nesting::Many(_) => (quote!(many), quote!(decode_many)),
				};

				let with_deleted = with_deleted(&args);
				Ok(Generated {
					selected: quote_spanned! { link.span()=>
						#private::Selected::from(#from::#via().#fetch::<#shape>() #(#refinements)* #with_deleted)
					},
					decoder: quote_spanned! { span=>
						#ident: #private::#decode::<#shape>(&mut values, #label, #field_label)?
					},
					nested: Some(shape),
				})
			}
		}
	}
}

/// The relation a link or backlink named `link` declared on its table.
fn via(link: &Ident) -> Ident {
	format_ident!("__via_{}", crate::table::unraw(link), span = link.span())
}

/// The calls that narrow a relation: `.filter(…)`, `.order_by(…)`, and
/// `.limit(…)` and `.offset(…)`, each spanned at its option.
fn refinements(args: &FieldArgs) -> Vec<TokenStream> {
	let mut calls = Vec::new();

	for filter in &args.filter {
		calls.push(quote_spanned! { filter.span()=> .filter(#filter) });
	}

	for order in &args.order_by {
		calls.push(quote_spanned! { order.span()=> .order_by(#order) });
	}

	if let Some(limit) = &args.limit {
		calls.push(quote_spanned! { limit.span()=> .limit(#limit) });
	}

	if let Some(offset) = &args.offset {
		calls.push(quote_spanned! { offset.span()=> .offset(#offset) });
	}

	calls
}

/// `.with_deleted()`, when the field asks for soft-deleted rows too. A
/// required nested row takes it, unlike the options that narrow rows: it
/// reads a soft-deleted row its link points at instead of failing.
fn with_deleted(args: &FieldArgs) -> Option<TokenStream> {
	args.with_deleted
		.map(|span| quote_spanned! { span=> .with_deleted() })
}

/// The type inside `wrapper<…>`, when `ty` is one.
fn wrapped<'a>(ty: &'a Type, wrapper: &str) -> Option<&'a Type> {
	let Type::Path(path) = ty else {
		return None;
	};
	let segment = path.path.segments.last()?;

	if segment.ident != wrapper {
		return None;
	}

	let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
		return None;
	};

	match arguments.args.first()? {
		syn::GenericArgument::Type(inner) => Some(inner),
		_ => None,
	}
}
