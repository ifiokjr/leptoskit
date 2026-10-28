//! Expansions and errors, checked by calling the expanders directly so that
//! the generated code can be reviewed as code.

use std::fmt::Write;

use proc_macro2::TokenStream;
use quote::quote;

use crate::attrs::TableArgs;

fn runtime() -> TokenStream {
	quote!(::leptoskit_db)
}

fn expand_table(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
	crate::table::expand(&TableArgs::parse(args)?, syn::parse2(item)?, &runtime())
}

/// Formats an expansion. The bodies of `__server!` and `__client!` are
/// formatted as items under a comment naming the gate, so that they read as
/// code rather than as a token stream.
fn pretty(tokens: TokenStream) -> String {
	let file: syn::File = syn::parse2(tokens).expect("the expansion parses as a file");
	let mut output = String::new();

	for item in file.items {
		if let Some((gate, body)) = gated(&item) {
			let _ = writeln!(output, "// ── {gate} ──\n{}", prettyplease::unparse(&body));
		} else {
			let single: syn::File = syn::parse2(quote!(#item)).expect("an item parses as a file");
			let _ = writeln!(output, "{}", prettyplease::unparse(&single));
		}
	}

	output
}

/// The gate's name and its items, when `item` is `__server! { ... }` or
/// `__client! { ... }`.
fn gated(item: &syn::Item) -> Option<(String, syn::File)> {
	let syn::Item::Macro(item) = item else {
		return None;
	};
	let gate = item.mac.path.segments.last()?.ident.to_string();

	if gate != "__server" && gate != "__client" {
		return None;
	}

	Some((
		gate,
		syn::parse2(item.mac.tokens.clone()).expect("the gated items parse"),
	))
}

fn table(args: TokenStream, item: TokenStream) -> String {
	pretty(expand_table(args, item).expect("the table expands"))
}

fn table_error(args: TokenStream, item: TokenStream) -> String {
	expand_table(args, item)
		.expect_err("the table is rejected")
		.to_string()
}

#[test]
fn a_public_table_with_every_column_option() {
	insta::assert_snapshot!(table(
		quote!(name = "users", public),
		quote! {
			/// Someone with an account.
			#[derive(Clone)]
			pub struct User {
				pub id: Id<Self>,
				/// Shown on profiles.
				#[db(unique(except = deleted_at), min_len = 3, max_len = 32, regex = "^[a-z0-9_]+$")]
				pub handle: String,
				#[db(private, column = "email_address")]
				pub email: String,
				#[db(secret)]
				pub password_hash: String,
				#[db(index, min = 0, max = 100, default = 0)]
				pub karma: i32,
				#[db(one_of("light", "dark"), default = "light", renamed_from = "colour_scheme")]
				pub theme: String,
				#[db(default = -1.5)]
				pub balance: f64,
				#[db(default(sql = "lower('x')"))]
				pub slug: String,
				#[db(on_delete = cascade)]
				pub invited_by: Option<Id<User>>,
				#[db(no_foreign_key)]
				pub legacy_id: Option<Id<User>>,
				#[db(created_at, default = now)]
				pub created_at: Timestamp,
				#[db(updated_at)]
				pub updated_at: Timestamp,
				#[db(deleted_at)]
				pub deleted_at: Option<Timestamp>,
				#[db(default = true)]
				pub active: bool,
			}
		},
	));
}

#[test]
fn a_private_table_with_a_composite_key_and_table_rules() {
	insta::assert_snapshot!(table(
		quote!(
			name = "post_tags",
			namespace = "content",
			index(tag, post_id),
			unique(post_id, position),
			check(name = "post_tags_position_check", sql = "position >= 0"),
			renamed_from = "taggings",
			crate = "::leptoskit_db"
		),
		quote! {
			pub(crate) struct PostTag {
				#[db(key)]
				pub post_id: Id<Post>,
				#[db(key, generate = false)]
				pub tag: String,
				pub position: i16,
			}
		},
	));
}

#[test]
fn a_named_public_type() {
	let expansion = table(
		quote!(name = "profiles", public = Profile),
		quote! {
			pub struct Account {
				pub id: Id<Account>,
				pub name: String,
			}
		},
	);
	let compact: String = expansion.split_whitespace().collect();
	assert!(compact.contains("pubstructProfile"));
	assert!(compact.contains("impl::core::convert::From<Account>forProfile"));
}

#[test]
fn enums_store_one_value_per_variant() {
	let input: syn::DeriveInput = syn::parse2(quote! {
		pub enum Role {
			Member,
			#[db(rename = "mod")]
			Moderator,
			SuperAdmin,
		}
	})
	.unwrap();
	insta::assert_snapshot!(pretty(crate::enums::expand(&input, &runtime()).unwrap()));
}

#[test]
fn table_mistakes_are_explained() {
	let user = quote! { pub struct User { pub id: Id<Self> } };
	let cases = [
		("no name", table_error(quote!(public), user.clone())),
		(
			"unknown table option",
			table_error(quote!(name = "users", wat), user.clone()),
		),
		(
			"check without sql",
			table_error(quote!(name = "users", check(name = "x")), user.clone()),
		),
		(
			"check with an unknown part",
			table_error(
				quote!(name = "users", check(name = "x", when = "y")),
				user.clone(),
			),
		),
		(
			"empty index",
			table_error(quote!(name = "users", index()), user.clone()),
		),
		(
			"derived Serialize",
			table_error(
				quote!(name = "users"),
				quote! { #[derive(Clone, serde::Serialize)] pub struct User { pub id: Id<Self> } },
			),
		),
		(
			"derived Debug",
			table_error(
				quote!(name = "users"),
				quote! { #[derive(Debug)] pub struct User { pub id: Id<Self> } },
			),
		),
		(
			"generic",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User<T> { pub id: Id<Self>, pub value: T } },
			),
		),
		(
			"tuple",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User(Id<User>); },
			),
		),
		(
			"no key",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User { pub name: String } },
			),
		),
		(
			"unknown column option",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User { pub id: Id<Self>, #[db(wat)] pub name: String } },
			),
		),
		(
			"bad regex",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User { pub id: Id<Self>, #[db(regex = "(")] pub name: String } },
			),
		),
		(
			"bad on_delete",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User { pub id: Id<Self>, #[db(on_delete = vanish)] pub boss: Id<User> } },
			),
		),
		(
			"bad default",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User { pub id: Id<Self>, #[db(default = later)] pub at: String } },
			),
		),
		(
			"bad default literal",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User { pub id: Id<Self>, #[db(default = b'x')] pub at: u8 } },
			),
		),
		(
			"default with an unknown part",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User { pub id: Id<Self>, #[db(default(expr = "x"))] pub at: String } },
			),
		),
		(
			"unique with an unknown part",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User { pub id: Id<Self>, #[db(unique(when = deleted_at))] pub name: String } },
			),
		),
		(
			"unique except a missing field",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User { pub id: Id<Self>, #[db(unique(except = gone))] pub name: String } },
			),
		),
		(
			"generated key that isn't Id<Self>",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User { #[db(key, generate = true)] pub name: String } },
			),
		),
		(
			"two created_at",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User { pub id: Id<Self>, #[db(created_at)] pub a: Timestamp, #[db(created_at)] pub b: Timestamp } },
			),
		),
		(
			"timestamp key",
			table_error(
				quote!(name = "users"),
				quote! { pub struct User { #[db(key, created_at)] pub at: Timestamp } },
			),
		),
	];

	let report: Vec<String> = cases
		.iter()
		.map(|(case, error)| format!("{case}: {error}"))
		.collect();
	insta::assert_snapshot!(report.join("\n"));
}

#[test]
fn enum_mistakes_are_explained() {
	let error = |tokens: TokenStream| {
		let input: syn::DeriveInput = syn::parse2(tokens).unwrap();
		crate::enums::expand(&input, &runtime())
			.unwrap_err()
			.to_string()
	};
	let cases = [
		("struct", error(quote! { struct Role; })),
		("data", error(quote! { enum Role { Member(u8) } })),
		("empty", error(quote! { enum Role {} })),
		(
			"generic",
			error(quote! { enum Role<T> { Member, Other(T) } }),
		),
		(
			"bad variant option",
			error(quote! { enum Role { #[db(value = "x")] Member } }),
		),
	];

	let report: Vec<String> = cases
		.iter()
		.map(|(case, error)| format!("{case}: {error}"))
		.collect();
	insta::assert_snapshot!(report.join("\n"));
}

#[test]
fn the_runtime_path_defaults_to_leptoskit_db_outside_an_app() {
	assert_eq!(
		crate::runtime(None).to_string(),
		quote!(::leptoskit_db).to_string()
	);

	let explicit: syn::Path = syn::parse_quote!(my_app::db);
	assert_eq!(
		crate::runtime(Some(&explicit)).to_string(),
		quote!(my_app::db).to_string()
	);
}
