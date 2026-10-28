//! Expands `#[derive(db::Scalar)]`: a newtype that is valid by construction,
//! stored as its inner type with the same rules as database checks.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Data;
use syn::DeriveInput;
use syn::Expr;
use syn::Fields;
use syn::Ident;
use syn::LitStr;
use syn::Type;
use syn::meta::ParseNestedMeta;

/// How a text scalar is normalized before it is checked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Normalize {
	Trim,
	Lowercase,
	Uppercase,
}

#[derive(Default)]
struct ScalarArgs {
	min_len: Option<Expr>,
	max_len: Option<Expr>,
	min: Option<Expr>,
	max: Option<Expr>,
	regex: Option<LitStr>,
	normalize: Vec<(Normalize, Ident)>,
}

impl ScalarArgs {
	fn parse(input: &DeriveInput) -> syn::Result<Self> {
		let mut args = Self::default();

		for attr in input.attrs.iter().filter(|attr| attr.path().is_ident("db")) {
			attr.parse_nested_meta(|meta| args.option(&meta))?;
		}

		Ok(args)
	}

	fn option(&mut self, meta: &ParseNestedMeta<'_>) -> syn::Result<()> {
		let path = &meta.path;

		if path.is_ident("min_len") {
			self.min_len = Some(meta.value()?.parse()?);
		} else if path.is_ident("max_len") {
			self.max_len = Some(meta.value()?.parse()?);
		} else if path.is_ident("min") {
			self.min = Some(meta.value()?.parse()?);
		} else if path.is_ident("max") {
			self.max = Some(meta.value()?.parse()?);
		} else if path.is_ident("regex") {
			let pattern: LitStr = meta.value()?.parse()?;

			if let Err(error) = regex::Regex::new(&pattern.value()) {
				return Err(syn::Error::new(
					pattern.span(),
					format!("`regex` isn't a valid regular expression: {error}"),
				));
			}

			self.regex = Some(pattern);
		} else if path.is_ident("normalize") {
			if meta.input.peek(syn::token::Paren) {
				meta.parse_nested_meta(|inner| {
					let ident = inner.path.require_ident()?.clone();
					self.normalize.push((normalization(&ident)?, ident));
					Ok(())
				})?;
			} else {
				let ident: Ident = meta.value()?.parse()?;
				self.normalize.push((normalization(&ident)?, ident));
			}
		} else {
			return Err(meta.error(
				"unknown scalar option; expected `min_len`, `max_len`, `min`, `max`, `regex`, or \
				 `normalize`",
			));
		}

		Ok(())
	}

	fn has_text_rules(&self) -> bool {
		self.min_len.is_some() || self.max_len.is_some() || self.regex.is_some()
	}
}

fn normalization(ident: &Ident) -> syn::Result<Normalize> {
	match ident.to_string().as_str() {
		"trim" => Ok(Normalize::Trim),
		"lowercase" => Ok(Normalize::Lowercase),
		"uppercase" => Ok(Normalize::Uppercase),
		_ => {
			Err(syn::Error::new(
				ident.span(),
				"`normalize` is `trim`, `lowercase`, or `uppercase`, or several of them: \
				 `normalize(trim, lowercase)`",
			))
		}
	}
}

pub(crate) fn expand(input: &DeriveInput, krate: &TokenStream) -> syn::Result<TokenStream> {
	let inner = inner_type(input)?;
	let args = ScalarArgs::parse(input)?;
	let text = is_string(inner);
	check_rules_fit(inner, &args, text)?;

	let name = &input.ident;
	let label = name.to_string();
	let normalize = args.normalize.iter().map(|(normalize, _)| {
		match normalize {
			Normalize::Trim => {
				quote! {
					let trimmed = value.trim();
					if trimmed.len() != value.len() {
						value = ::std::string::String::from(trimmed);
					}
				}
			}
			Normalize::Lowercase => quote! { value = value.to_lowercase(); },
			Normalize::Uppercase => quote! { value = value.to_uppercase(); },
		}
	});
	let checks = checks(krate, &label, &args);
	let rules = rules(krate, &args);
	let schema = schema_rules(&args);
	let schemars_path = format!("{krate}::__private::schemars").replace(' ', "");
	let text_impls = text.then(|| {
		quote! {
			impl ::core::str::FromStr for #name {
				type Err = #krate::ValidationErrors;

				fn from_str(value: &str) -> ::core::result::Result<Self, Self::Err> {
					Self::parse(value)
				}
			}

			impl ::core::convert::AsRef<str> for #name {
				fn as_ref(&self) -> &str {
					&self.0
				}
			}
		}
	});
	let text_column = text.then(|| quote! { impl #krate::TextColumn for #name {} });
	let normalize_mut = (!args.normalize.is_empty()).then(|| quote!(mut));
	// Text takes anything that becomes a `String`; a number takes exactly its
	// type, so that a literal such as `4` infers it.
	let (parameter, conversion) = if text {
		(
			quote!(impl ::core::convert::Into<#inner>),
			quote!(::core::convert::Into::into(value)),
		)
	} else {
		(quote!(#inner), quote!(value))
	};

	Ok(quote! {
		impl #name {
			/// Normalizes the value and checks every rule the scalar declares:
			/// a value of this type that exists is valid.
			pub fn parse(value: #parameter) -> ::core::result::Result<Self, #krate::ValidationErrors> {
				let #normalize_mut value: #inner = #conversion;
				#(#normalize)*
				#checks
				::core::result::Result::Ok(Self(value))
			}

			/// The checked value.
			pub fn as_inner(&self) -> &#inner {
				&self.0
			}

			/// The checked value, owned.
			pub fn into_inner(self) -> #inner {
				self.0
			}
		}

		impl ::core::ops::Deref for #name {
			type Target = #inner;

			fn deref(&self) -> &#inner {
				&self.0
			}
		}

		impl ::core::fmt::Display for #name {
			fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
				::core::fmt::Display::fmt(&self.0, formatter)
			}
		}

		impl ::core::convert::TryFrom<#inner> for #name {
			type Error = #krate::ValidationErrors;

			fn try_from(value: #inner) -> ::core::result::Result<Self, Self::Error> {
				Self::parse(value)
			}
		}

		impl ::core::convert::From<#name> for #inner {
			fn from(value: #name) -> Self {
				value.0
			}
		}

		#text_impls

		impl #krate::__private::serde::Serialize for #name {
			fn serialize<S: #krate::__private::serde::Serializer>(&self, serializer: S) -> ::core::result::Result<S::Ok, S::Error> {
				#krate::__private::serde::Serialize::serialize(&self.0, serializer)
			}
		}

		impl<'de> #krate::__private::serde::Deserialize<'de> for #name {
			fn deserialize<D: #krate::__private::serde::Deserializer<'de>>(deserializer: D) -> ::core::result::Result<Self, D::Error> {
				let value = <#inner as #krate::__private::serde::Deserialize>::deserialize(deserializer)?;
				Self::parse(value).map_err(<D::Error as #krate::__private::serde::de::Error>::custom)
			}
		}

		const _: () = {
			/// The scalar's JSON Schema: its inner type's, with its rules.
			#[derive(#krate::__private::schemars::JsonSchema)]
			#[schemars(crate = #schemars_path)]
			#[allow(dead_code, reason = "only its schema is used")]
			struct Schema(#schema #inner);

			impl #krate::__private::schemars::JsonSchema for #name {
				fn schema_name() -> ::std::borrow::Cow<'static, str> {
					::std::borrow::Cow::Borrowed(#label)
				}

				fn json_schema(generator: &mut #krate::__private::schemars::SchemaGenerator) -> #krate::__private::schemars::Schema {
					<Schema as #krate::__private::schemars::JsonSchema>::json_schema(generator)
				}
			}
		};

		#krate::__server! {
			impl #krate::Column for #name {
				fn column_type() -> #krate::__private::schema::ColumnType {
					<#inner as #krate::Column>::column_type()
				}

				fn rules() -> #krate::ScalarRules {
					#rules
				}

				fn to_value(&self) -> ::core::result::Result<#krate::Value, #krate::EncodeError> {
					#krate::Column::to_value(&self.0)
				}

				fn from_value(value: #krate::Value) -> ::core::result::Result<Self, #krate::DecodeError> {
					let value = <#inner as #krate::Column>::from_value(value)?;
					Self::parse(value).map_err(#krate::DecodeError::custom)
				}
			}

			#text_column
		}
	})
}

/// Text rules need a `String`, and number rules need a number.
fn check_rules_fit(inner: &Type, args: &ScalarArgs, text: bool) -> syn::Result<()> {
	if text {
		if args.min.is_some() || args.max.is_some() {
			return Err(syn::Error::new_spanned(
				inner,
				"`min` and `max` compare numbers, so the scalar must wrap a number",
			));
		}

		return Ok(());
	}

	if let Some(rule) = args
		.min_len
		.as_ref()
		.map(|_| "min_len")
		.or(args.max_len.as_ref().map(|_| "max_len"))
		.or(args.regex.as_ref().map(|_| "regex"))
	{
		return Err(syn::Error::new_spanned(
			inner,
			format!("`{rule}` checks text, so the scalar must wrap a `String`"),
		));
	}

	if let Some((_, ident)) = args.normalize.first() {
		return Err(syn::Error::new(
			ident.span(),
			"`normalize` changes text, so the scalar must wrap a `String`",
		));
	}

	Ok(())
}

/// The single field of a tuple struct with one field.
fn inner_type(input: &DeriveInput) -> syn::Result<&Type> {
	const SHAPE: &str = "a scalar wraps one value: `struct Handle(String);`";

	if !input.generics.params.is_empty() {
		return Err(syn::Error::new_spanned(
			&input.generics,
			"a scalar can't be generic",
		));
	}

	let Data::Struct(data) = &input.data else {
		return Err(syn::Error::new_spanned(&input.ident, SHAPE));
	};
	let Fields::Unnamed(fields) = &data.fields else {
		return Err(syn::Error::new_spanned(&input.ident, SHAPE));
	};

	match fields.unnamed.first() {
		Some(field) if fields.unnamed.len() == 1 => Ok(&field.ty),
		_ => Err(syn::Error::new_spanned(&data.fields, SHAPE)),
	}
}

fn is_string(ty: &Type) -> bool {
	matches!(ty, Type::Path(path) if path.qself.is_none() && path.path.segments.last().is_some_and(|segment| segment.ident == "String"))
}

/// The checks `parse` runs, returning early with every broken rule.
fn checks(krate: &TokenStream, label: &str, args: &ScalarArgs) -> TokenStream {
	let mut checks = Vec::new();

	if args.min_len.is_some() || args.max_len.is_some() {
		let min = option_cast(args.min_len.as_ref(), &quote!(u32));
		let max = option_cast(args.max_len.as_ref(), &quote!(u32));
		checks.push(
			quote! { #krate::__private::rules::length(&mut errors, #label, &value, #min, #max); },
		);
	}

	if args.min.is_some() || args.max.is_some() {
		let min = option_cast(args.min.as_ref(), &quote!(f64));
		let max = option_cast(args.max.as_ref(), &quote!(f64));
		checks.push(
			quote! { #krate::__private::rules::range(&mut errors, #label, &value, #min, #max); },
		);
	}

	if let Some(pattern) = &args.regex {
		checks.push(
			quote! { #krate::__private::rules::pattern(&mut errors, #label, &value, #pattern); },
		);
	}

	if checks.is_empty() {
		return quote! {};
	}

	quote! {
		let mut errors = #krate::ValidationErrors::default();
		#(#checks)*
		errors.into_result()?;
	}
}

/// The same rules as database checks.
fn rules(krate: &TokenStream, args: &ScalarArgs) -> TokenStream {
	let min_len = option_cast(args.min_len.as_ref(), &quote!(u32));
	let max_len = option_cast(args.max_len.as_ref(), &quote!(u32));
	let min = option_cast(args.min.as_ref(), &quote!(f64));
	let max = option_cast(args.max.as_ref(), &quote!(f64));
	let pattern = args.regex.as_ref().map_or_else(
		|| quote! { ::core::option::Option::None },
		|pattern| quote! { ::core::option::Option::Some(#pattern) },
	);

	quote! {
		#krate::ScalarRules {
			min_len: #min_len,
			max_len: #max_len,
			min: #min,
			max: #max,
			pattern: #pattern,
		}
	}
}

/// The rules as `schemars` attributes, so generated clients know them.
fn schema_rules(args: &ScalarArgs) -> TokenStream {
	let mut rules = Vec::new();

	if args.has_text_rules() {
		let min = args.min_len.as_ref().map(|min| quote!(min = #min));
		let max = args.max_len.as_ref().map(|max| quote!(max = #max));

		if min.is_some() || max.is_some() {
			let parts = min.into_iter().chain(max);
			rules.push(quote! { #[schemars(length(#(#parts),*))] });
		}
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

fn option_cast(value: Option<&Expr>, ty: &TokenStream) -> TokenStream {
	value.map_or_else(
		|| quote! { ::core::option::Option::None },
		|value| quote! { ::core::option::Option::Some((#value) as #ty) },
	)
}
