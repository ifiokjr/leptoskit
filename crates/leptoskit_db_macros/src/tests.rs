//! Expansions and errors, checked by calling the expanders directly so that
//! the generated code can be reviewed as code.
#![expect(
	clippy::needless_pass_by_value,
	reason = "the cases pass `quote!` output straight to the helpers"
)]

use std::fmt::Write;

use proc_macro2::TokenStream;
use quote::quote;

fn runtime() -> TokenStream {
	quote!(::leptoskit_db)
}

fn expand_table(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
	crate::table::expand(&args, &item, &[])
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
		(
			"unknown merge strategy",
			table_error(
				quote!(name = "users", on_account_merge = shuffle),
				quote! { pub struct User { pub id: Id<Self> } },
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
fn link_mistakes_are_explained() {
	let cases = [
		(
			"link named like a method",
			table_error(
				quote!(name = "posts"),
				quote! { pub struct Post { pub id: Id<Self>, pub update_id: Id<Update> } },
			),
		),
		(
			"two links with one name",
			table_error(
				quote!(name = "posts"),
				quote! { pub struct Post { pub id: Id<Self>, pub author: Id<User>, pub author_id: Id<User> } },
			),
		),
		(
			"soft cascade on a plain column",
			table_error(
				quote!(name = "posts"),
				quote! { pub struct Post { pub id: Id<Self>, #[db(on_soft_delete = cascade)] pub title: String, #[db(deleted_at)] pub at: Option<Timestamp> } },
			),
		),
		(
			"soft cascade without soft delete",
			table_error(
				quote!(name = "posts"),
				quote! { pub struct Post { pub id: Id<Self>, #[db(on_soft_delete = cascade)] pub author_id: Id<User> } },
			),
		),
		(
			"soft cascade action",
			table_error(
				quote!(name = "posts"),
				quote! { pub struct Post { pub id: Id<Self>, #[db(on_soft_delete = set_null)] pub author_id: Id<User> } },
			),
		),
		(
			"backlink on a plain column",
			table_error(
				quote!(name = "posts"),
				quote! { pub struct Post { pub id: Id<Self>, #[db(backlink = posts)] pub title: String } },
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
fn pack_tables_are_namespaced_and_written_only_by_their_pack() {
	let expansion = table(
		quote!(name = "sessions", pack = "auth"),
		quote! {
			pub struct Session {
				pub id: Id<Self>,
				pub user_agent: String,
			}
		},
	);
	let compact: String = expansion.split_whitespace().collect();
	assert!(
		compact.contains(
			"constNAMESPACE:::core::option::Option<&'staticstr>=::core::option::Option::Some(\"\
			 auth\",);"
		),
		"{expansion}"
	);
	assert!(
		compact.contains(
			"constPACK:::core::option::Option<&'staticstr>=::core::option::Option::Some(\"auth\",\
			 );"
		),
		"{expansion}"
	);
	for write in [
		"pub(crate)fnupdate(",
		"pub(crate)asyncfnsave(",
		"pub(crate)asyncfndelete(",
		"pub(crate)asyncfndestroy(",
		"pub(crate)asyncfninsert(",
		"pub(crate)asyncfnupsert<",
	] {
		assert!(compact.contains(write), "{write} in {expansion}");
	}
	assert!(compact.contains("pubfnquery()"), "reads stay public");
}

#[test]
fn policies_become_the_tables_rules() {
	let expansion = table(
		quote!(name = "posts"),
		quote! {
			#[db::policy(select, |post, cx| post.published.eq(true))]
			#[policy(insert, update, |post, cx| cx.global(|viewer: &Viewer| post.author_id.eq(viewer.0)), message = "Only the author")]
			#[db::policy(deny delete, |post, _| post.locked.eq(true))]
			#[db::policy(all, |_, cx| cx.global(|_: &Admin| Expr::always()))]
			pub struct Post {
				pub id: Id<Self>,
				pub author_id: Id<User>,
				pub published: bool,
				pub locked: bool,
			}
		},
	);
	insta::assert_snapshot!(expansion);
}

#[test]
fn a_misplaced_policy_says_where_it_goes() {
	let expansion = crate::table::misplaced_policy(quote!(
		pub struct Post;
	))
	.to_string();
	assert!(
		expansion.contains("put `#[db::policy]` below `#[db::table]`, which reads it"),
		"{expansion}"
	);
	assert!(expansion.ends_with("pub struct Post ;"), "{expansion}");
}

#[test]
fn policy_mistakes_are_explained() {
	let error = |policy: TokenStream| {
		table_error(
			quote!(name = "posts"),
			quote! {
				#[db::policy(#policy)]
				pub struct Post { pub id: Id<Self> }
			},
		)
	};
	let cases = [
		("unknown action", error(quote!(read, |p, cx| x))),
		("no actions", error(quote!(|p, cx| x))),
		("one parameter", error(quote!(select, |p| x))),
		(
			"unknown option",
			error(quote!(select, |p, cx| x, reason = "no")),
		),
		("missing comma", error(quote!(select | p, cx | x))),
	];

	let report: Vec<String> = cases
		.iter()
		.map(|(case, error)| format!("{case}: {error}"))
		.collect();
	insta::assert_snapshot!(report.join("\n"));
}

fn mixin(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
	crate::mixin::expand(&args, syn::parse2(item).unwrap())
}

#[test]
fn mixins_declare_their_public_and_insert_types_and_a_callback() {
	let expansion = mixin(
		quote!(index(by)),
		quote! {
			/// Who added a row.
			pub struct Stamp {
				/// Who.
				#[db(max_len = 40)]
				pub by: String,
				#[db(private)]
				pub note: Option<String>,
				#[db(created_at)]
				pub at: Timestamp,
			}
		},
	)
	.unwrap();
	// The callback's name carries a hash of the declaration; the snapshot
	// pins everything around it.
	let text = expansion.to_string();
	let hash = text
		.split("__leptoskit_db_mixin_Stamp_")
		.nth(1)
		.and_then(|rest| rest.split_whitespace().next())
		.unwrap()
		.to_string();
	let masked: TokenStream = text.replace(&hash, "HASH").parse().unwrap();
	insta::assert_snapshot!(pretty(masked));
}

#[test]
fn a_flattened_table_asks_each_mixin_then_expands() {
	let args = quote!(name = "orders", public);
	let item = quote! {
		pub struct Order {
			pub id: Id<Self>,
			#[db(flatten, prefix = "billing_")]
			pub billing: Address,
			#[db(flatten)]
			pub times: db::Timestamps,
		}
	};

	// No descriptions yet: ask the first mixin.
	let asked = crate::table::expand(&args, &item, &[]).unwrap().to_string();
	assert!(
		asked.starts_with("Address ! { (:: leptoskit_db :: __private :: table_continue) ["),
		"{asked}"
	);

	// One description: ask the second.
	let address = quote! {
		[index(city)]
		[#[db(max_len = 100)] street; city; #[db(private)] notes;]
	};
	let asked = crate::table::expand(&args, &item, std::slice::from_ref(&address))
		.unwrap()
		.to_string();
	assert!(asked.starts_with("db :: Timestamps !"), "{asked}");

	// Both: expand.
	let timestamps = quote! {
		[]
		[#[db(created_at)] created_at; #[db(updated_at)] updated_at;]
	};
	insta::assert_snapshot!(pretty(
		crate::table::expand(&args, &item, &[address, timestamps]).unwrap()
	));
}

#[test]
fn mixin_mistakes_are_explained() {
	let error = |args: TokenStream, item: TokenStream| mixin(args, item).unwrap_err().to_string();
	let table_error = |item: TokenStream, descriptions: &[TokenStream]| {
		crate::table::expand(&quote!(name = "t"), &item, descriptions)
			.unwrap_err()
			.to_string()
	};
	let times = quote! { [] [#[db(created_at)] created_at;] };
	let cases = [
		(
			"generic",
			error(quote!(), quote! { struct Pair<T> { a: T } }),
		),
		("tuple", error(quote!(), quote! { struct Pair(i32); })),
		(
			"key",
			error(quote!(), quote! { struct Keyed { #[db(key)] a: i32 } }),
		),
		(
			"nested",
			error(
				quote!(),
				quote! { struct Outer { #[db(flatten)] inner: Inner } },
			),
		),
		(
			"derive",
			error(
				quote!(),
				quote! { #[derive(Debug)] struct Times { a: i32 } },
			),
		),
		(
			"unknown index field",
			error(quote!(index(missing)), quote! { struct Times { a: i32 } }),
		),
		(
			"unknown option",
			error(quote!(name = "x"), quote! { struct Times { a: i32 } }),
		),
		(
			"generic mixin type",
			table_error(
				quote! { struct T { id: Id<Self>, #[db(flatten)] times: Wrapper<i32> } },
				&[],
			),
		),
		(
			"duplicate column",
			table_error(
				quote! { struct T { id: Id<Self>, created_at: i32, #[db(flatten)] times: Times } },
				std::slice::from_ref(&times),
			),
		),
		(
			"two created_at",
			table_error(
				quote! {
					struct T {
						id: Id<Self>,
						#[db(flatten)] times: Times,
						#[db(flatten, prefix = "other_")] other: Times,
					}
				},
				&[times.clone(), times.clone()],
			),
		),
		(
			"prefix without flatten",
			table_error(
				quote! { struct T { id: Id<Self>, #[db(prefix = "a_")] a: i32 } },
				&[],
			),
		),
		(
			"flattened key",
			table_error(
				quote! { struct T { #[db(key, flatten)] times: Times } },
				std::slice::from_ref(&times),
			),
		),
		(
			"bad description",
			table_error(
				quote! { struct T { id: Id<Self>, #[db(flatten)] times: Times } },
				&[quote! { [] [created_at] }],
			),
		),
		(
			"bad continuation",
			crate::table::expand_continued(quote! { (x) })
				.unwrap_err()
				.to_string(),
		),
	];

	let report: Vec<String> = cases
		.iter()
		.map(|(case, error)| format!("{case}: {error}"))
		.collect();
	insta::assert_snapshot!(report.join("\n"));
}

fn scalar(tokens: TokenStream) -> syn::Result<TokenStream> {
	let input: syn::DeriveInput = syn::parse2(tokens).unwrap();
	crate::scalar::expand(&input, &runtime())
}

#[test]
fn text_scalars_normalize_then_check() {
	insta::assert_snapshot!(pretty(
		scalar(quote! {
			#[db(min_len = 4, max_len = 20, regex = "^[a-z0-9_]+$", normalize(trim, lowercase))]
			pub struct Handle(String);
		})
		.unwrap()
	));
}

#[test]
fn number_scalars_check_their_range() {
	insta::assert_snapshot!(pretty(
		scalar(quote! {
			#[db(min = 1, max = 5)]
			pub struct Stars(i16);
		})
		.unwrap()
	));
}

#[test]
fn scalar_mistakes_are_explained() {
	let error = |tokens: TokenStream| scalar(tokens).unwrap_err().to_string();
	let cases = [
		("unit", error(quote! { struct Handle; })),
		("named", error(quote! { struct Handle { value: String } })),
		("two fields", error(quote! { struct Pair(String, String); })),
		("enum", error(quote! { enum Handle { A } })),
		("generic", error(quote! { struct Wrapper<T>(T); })),
		(
			"length on a number",
			error(quote! { #[db(max_len = 3)] struct Stars(i16); }),
		),
		(
			"normalize a number",
			error(quote! { #[db(normalize = trim)] struct Stars(i16); }),
		),
		(
			"range on text",
			error(quote! { #[db(min = 1)] struct Handle(String); }),
		),
		(
			"unknown normalization",
			error(quote! { #[db(normalize(reverse))] struct Handle(String); }),
		),
		(
			"bad regex",
			error(quote! { #[db(regex = "(")] struct Handle(String); }),
		),
		(
			"unknown option",
			error(quote! { #[db(one_of("a"))] struct Handle(String); }),
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

fn migration_dir(registry: &str, migrations: &[(&str, &str)]) -> tempfile::TempDir {
	let directory = tempfile::tempdir().unwrap();

	if !registry.is_empty() {
		std::fs::write(directory.path().join("registry.txt"), registry).unwrap();
	}

	for (name, sql) in migrations {
		std::fs::create_dir_all(directory.path().join(name)).unwrap();

		if !sql.is_empty() {
			std::fs::write(directory.path().join(name).join("migration.sql"), sql).unwrap();
		}
	}

	directory
}

fn expand_migrations(directory: &tempfile::TempDir) -> syn::Result<TokenStream> {
	let path = syn::LitStr::new(".", proc_macro2::Span::call_site());
	crate::migrations::expand(&path, directory.path(), &runtime())
}

#[test]
fn migrations_embed_in_registry_order_with_checksums() {
	let directory = migration_dir(
		"# order\nsecond\n\nfirst\n",
		&[("first", "SELECT 1;"), ("second", "SELECT 2;")],
	);
	let expansion = expand_migrations(&directory).unwrap().to_string();
	let second = expansion.find("\"second\"").unwrap();
	let first = expansion.find("\"first\"").unwrap();

	assert!(second < first, "registry order: {expansion}");
	assert!(expansion.contains("include_str"));
	// The SHA-256 of `SELECT 1;`.
	assert!(
		expansion.contains("\"17db4fd369edb9244b9f91d9aeed145c3d04ad8ba6e95d06247f07a63527d11a\"")
	);
}

#[test]
fn migration_mistakes_are_explained() {
	let missing_registry = migration_dir("", &[]);
	let unlisted = migration_dir("first\n", &[("first", "SELECT 1;"), ("stray", "SELECT 2;")]);
	let missing_sql = migration_dir("first\n", &[("first", "")]);
	let errors = [
		expand_migrations(&missing_registry)
			.unwrap_err()
			.to_string(),
		expand_migrations(&unlisted).unwrap_err().to_string(),
		expand_migrations(&missing_sql).unwrap_err().to_string(),
	];

	assert!(
		errors[0].contains("registry.txt") && errors[0].contains("leptoskit db migration create")
	);
	assert!(errors[1].starts_with("migration `stray` isn't listed in registry.txt"));
	assert!(errors[2].starts_with("registry.txt lists `first`, but "));
}
