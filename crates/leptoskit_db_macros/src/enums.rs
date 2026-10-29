//! Expands `#[derive(db::Enum)]`: a unit enum stored as text, one value per
//! variant.

use convert_case::Case;
use convert_case::Converter;
use proc_macro2::TokenStream;
use quote::quote;
use syn::Data;
use syn::DeriveInput;
use syn::Fields;
use syn::LitStr;

pub(crate) fn expand(input: &DeriveInput, krate: &TokenStream) -> syn::Result<TokenStream> {
	let Data::Enum(data) = &input.data else {
		return Err(syn::Error::new_spanned(
			&input.ident,
			"`db::Enum` is for enums with unit variants",
		));
	};

	if !input.generics.params.is_empty() {
		return Err(syn::Error::new_spanned(
			&input.generics,
			"a database enum can't be generic",
		));
	}

	let name = &input.ident;
	let mut idents = Vec::new();
	let mut values = Vec::new();

	for variant in &data.variants {
		if !matches!(variant.fields, Fields::Unit) {
			return Err(syn::Error::new_spanned(
				variant,
				"database enums store one text value per variant, so variants can't hold data",
			));
		}

		let mut value = None;

		for attr in variant
			.attrs
			.iter()
			.filter(|attr| attr.path().is_ident("db"))
		{
			attr.parse_nested_meta(|meta| {
				if meta.path.is_ident("rename") {
					value = Some(meta.value()?.parse::<LitStr>()?.value());
					Ok(())
				} else {
					Err(meta.error("a variant takes `rename = \"...\"`"))
				}
			})?;
		}

		idents.push(&variant.ident);
		values.push(value.unwrap_or_else(|| {
			Converter::new()
				.from_case(Case::Pascal)
				.to_case(Case::Snake)
				.convert(variant.ident.to_string())
		}));
	}

	if idents.is_empty() {
		return Err(syn::Error::new_spanned(
			name,
			"a database enum needs at least one variant",
		));
	}

	let label = name.to_string();
	let expected = values.join("`, `");

	Ok(quote! {
		impl #name {
			/// The value stored for each variant, in declaration order.
			pub const VALUES: &'static [&'static str] = &[#(#values),*];

			/// The value stored for this variant.
			pub const fn as_str(&self) -> &'static str {
				match self {
					#(Self::#idents => #values,)*
				}
			}
		}

		impl ::core::fmt::Display for #name {
			fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
				formatter.write_str(self.as_str())
			}
		}

		impl ::core::str::FromStr for #name {
			type Err = #krate::UnknownVariant;

			fn from_str(value: &str) -> ::core::result::Result<Self, Self::Err> {
				match value {
					#(#values => ::core::result::Result::Ok(Self::#idents),)*
					other => ::core::result::Result::Err(#krate::UnknownVariant {
						enumeration: #label,
						value: ::std::string::String::from(other),
						expected: #expected,
					}),
				}
			}
		}

		impl #krate::__private::serde::Serialize for #name {
			fn serialize<S: #krate::__private::serde::Serializer>(&self, serializer: S) -> ::core::result::Result<S::Ok, S::Error> {
				serializer.serialize_str(self.as_str())
			}
		}

		impl<'de> #krate::__private::serde::Deserialize<'de> for #name {
			fn deserialize<D: #krate::__private::serde::Deserializer<'de>>(deserializer: D) -> ::core::result::Result<Self, D::Error> {
				let value = <::std::string::String as #krate::__private::serde::Deserialize>::deserialize(deserializer)?;
				value.parse().map_err(<D::Error as #krate::__private::serde::de::Error>::custom)
			}
		}

		impl #krate::__private::schemars::JsonSchema for #name {
			fn schema_name() -> ::std::borrow::Cow<'static, str> {
				::std::borrow::Cow::Borrowed(#label)
			}

			fn json_schema(_: &mut #krate::__private::schemars::SchemaGenerator) -> #krate::__private::schemars::Schema {
				#krate::__private::schemars::json_schema!({ "type": "string", "enum": [#(#values),*] })
			}
		}

		#krate::__server! {
			impl #krate::Column for #name {
				fn column_type() -> #krate::__private::schema::ColumnType {
					#krate::__private::schema::ColumnType::TEXT
				}

				fn allowed_values() -> ::core::option::Option<&'static [&'static str]> {
					::core::option::Option::Some(Self::VALUES)
				}

				fn to_value(&self) -> ::core::result::Result<#krate::Value, #krate::EncodeError> {
					::core::result::Result::Ok(#krate::Value::Text(::std::string::String::from(self.as_str())))
				}

				fn from_value(value: #krate::Value) -> ::core::result::Result<Self, #krate::DecodeError> {
					match value {
						#krate::Value::Text(text) => text.parse().map_err(#krate::DecodeError::custom),
						other => ::core::result::Result::Err(#krate::DecodeError::unexpected("text", &other)),
					}
				}
			}
		}
	})
}
