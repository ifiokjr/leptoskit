//! Expansion snapshots in both registration modes, and the diagnostics
//! `#[api]` gives for what no generated client could call.

use proc_macro2::TokenStream;
use quote::ToTokens;
use quote::quote;

use super::argument_docs;
use super::expand;

/// Pretty-prints an expansion. prettyplease leaves macro bodies as raw
/// tokens, so the registration — an `inventory::submit!` of one descriptor —
/// is unfolded into `static REGISTERED: ApiFnDescriptor = ...;` to read as
/// Rust.
fn pretty(tokens: TokenStream) -> String {
	let mut file = syn::parse2::<syn::File>(tokens).expect("valid Rust");
	for item in &mut file.items {
		let syn::Item::Macro(registration) = item else {
			continue;
		};
		let path = registration.mac.path.to_token_stream().to_string();
		assert!(
			path.ends_with("__private :: inventory :: submit"),
			"registrations go through the re-exported inventory: {path}"
		);
		let descriptor: syn::ExprStruct = registration.mac.parse_body().expect("one descriptor");
		let ty = &descriptor.path;
		*item = syn::parse_quote!(static REGISTERED: #ty = #descriptor;);
	}
	prettyplease::unparse(&file)
}

/// The server build's expansion: the rewritten function and its
/// registration. Also checks the client build expands to the same function
/// without the registration.
fn expansion(args: TokenStream, item: TokenStream) -> String {
	let server = expand(args.clone(), item.clone(), true).expect("the server build expands");
	let client =
		syn::parse2::<syn::File>(expand(args, item, false).expect("the client build expands"))
			.expect("valid Rust");
	assert_eq!(
		client.items.len(),
		1,
		"the client build only rewrites the function"
	);
	let server_function = syn::parse2::<syn::File>(server.clone())
		.expect("valid Rust")
		.items[0]
		.to_token_stream()
		.to_string();
	assert_eq!(
		client.items[0].to_token_stream().to_string(),
		server_function,
		"both builds rewrite the function the same way"
	);
	pretty(server)
}

/// The error `#[api(args)]` gives on `item`, both parsed from source so the
/// span points at real text: the message, and the text the span covers.
fn error(args: &str, item: &str) -> (String, String) {
	let args: TokenStream = args.parse().expect("argument tokens");
	let item: TokenStream = item.parse().expect("item tokens");
	let error = expand(args, item, true).expect_err("an error");
	let underlined = error.span().source_text().unwrap_or_default();
	(error.to_string(), underlined)
}

#[test]
fn bare_server_functions_get_the_function_name_as_endpoint() {
	insta::assert_snapshot!(expansion(
		TokenStream::new(),
		quote! {
			/// Lists todos.
			#[server]
			pub async fn list_todos(
				filter: TodoFilter,
				limit: Option<u32>,
			) -> Result<Vec<Todo>, ServerFnError> {
				Ok(Vec::new())
			}
		}
	));
}

#[test]
fn client_builds_register_nothing() {
	let output = pretty(
		expand(
			TokenStream::new(),
			quote! {
				#[server]
				pub async fn ping() -> Result<(), ServerFnError> {
					Ok(())
				}
			},
			false,
		)
		.expect("expands"),
	);
	insta::assert_snapshot!(output);
	assert!(!output.contains("inventory"), "{output}");
}

#[test]
fn a_prefix_keeps_its_place_and_gains_an_endpoint() {
	insta::assert_snapshot!(expansion(
		TokenStream::new(),
		quote! {
			#[server(prefix = "/v1", input = GetUrl)]
			pub async fn get_todo(id: u64) -> Result<Option<Todo>, ServerFnError> {
				Ok(None)
			}
		}
	));
}

#[test]
fn a_trailing_comma_does_not_double_up() {
	let output = expansion(
		TokenStream::new(),
		quote! {
			#[server(prefix = "/v1",)]
			pub async fn ping() -> Result<(), ServerFnError> {
				Ok(())
			}
		},
	);
	assert!(
		output.contains(r#"#[server(prefix = "/v1", endpoint = "ping")]"#),
		"{output}"
	);
	insta::assert_snapshot!(output);
}

#[test]
fn explicit_endpoints_are_kept() {
	insta::assert_snapshot!(expansion(
		TokenStream::new(),
		quote! {
			#[leptos::server(prefix = "/v1", endpoint = "events", protocol = Http<PostUrl, Json>)]
			pub async fn old_events(since: u64) -> Result<Vec<Event>, ServerFnError> {
				Ok(Vec::new())
			}
		}
	));
}

#[test]
fn positional_attributes_with_an_endpoint_are_left_alone() {
	insta::assert_snapshot!(expansion(
		TokenStream::new(),
		quote! {
			#[server(LegacyPing, "/legacy", "GetJson", "ping")]
			pub async fn legacy_ping() -> Result<u8, ServerFnError> {
				Ok(1)
			}
		}
	));
}

#[test]
fn options_name_tag_and_crate() {
	let output = expansion(
		quote!(
			name = "events_since",
			tag = "events",
			tag = "legacy",
			crate = "leptoskit::rpc"
		),
		quote! {
			#[server]
			pub async fn old_events(since: u64) -> Result<Vec<Event>, ServerFnError> {
				Ok(Vec::new())
			}
		},
	);
	assert!(
		output.contains(r#"#[server(endpoint = "events_since")]"#),
		"`name` sets the route: {output}"
	);
	insta::assert_snapshot!(output);
}

#[test]
fn internal_functions_say_so() {
	let output = expansion(
		quote!(internal),
		quote! {
			#[server]
			pub async fn flush_caches() -> Result<(), ServerFnError> {
				Ok(())
			}
		},
	);
	assert!(output.contains("internal: true"), "{output}");
}

#[test]
fn the_arguments_section_moves_onto_the_arguments() {
	insta::assert_snapshot!(expansion(
		TokenStream::new(),
		quote! {
			/// Lists todos.
			///
			/// Newest first.
			///
			/// # Arguments
			///
			/// * `filter` - Which todos to list.
			/// * `limit` - How many to return; the server caps it
			///   at a hundred.
			/// * `type` - Loses to the doc comment on the argument.
			///
			/// # Errors
			///
			/// When the database is down.
			#[server]
			pub async fn list_todos(
				filter: TodoFilter,
				limit: Option<u32>,
				/// Overrides the section.
				r#type: Option<String>,
			) -> Result<Vec<Todo>, ServerFnError> {
				Ok(Vec::new())
			}
		}
	));
}

#[test]
fn argument_attributes_default_rename_and_skip() {
	let output = expansion(
		TokenStream::new(),
		quote! {
			#[server(input = Json)]
			pub async fn create_todo(
				title: String,
				#[server(default)] tags: Vec<String>,
				#[server(default = "Vec::new")] notes: Vec<String>,
				#[server(rename = "dueAt")] due_at: Option<u64>,
				#[server(skip)] trace: Option<String>,
			) -> Result<Todo, ServerFnError> {
				unimplemented!()
			}
		},
	);
	assert!(
		!output.contains(r#"name: "trace""#),
		"skipped arguments are not published: {output}"
	);
	insta::assert_snapshot!(output);
}

#[test]
fn deprecated_functions_say_so() {
	let output = expansion(
		TokenStream::new(),
		quote! {
			#[deprecated = "use the events stream"]
			#[server]
			pub async fn old_events() -> Result<(), ServerFnError> {
				Ok(())
			}
		},
	);
	assert!(output.contains("deprecated: true"), "{output}");
	insta::assert_snapshot!(output);
}

#[test]
fn raw_argument_names_are_unraw_on_the_wire() {
	let output = expansion(
		TokenStream::new(),
		quote! {
			#[server]
			pub async fn find(r#match: String) -> Result<(), ServerFnError> {
				Ok(())
			}
		},
	);
	assert!(output.contains(r#"name: "match""#), "{output}");
	assert!(!output.contains("r#match\""), "{output}");
}

#[test]
fn custom_wrappers_register_the_wrapped_struct() {
	let output = expansion(
		TokenStream::new(),
		quote! {
			#[server(custom = Traced)]
			pub async fn ping() -> Result<(), ServerFnError> {
				Ok(())
			}
		},
	);
	assert!(
		output.contains("<Traced<Ping> as ::leptoskit_rpc::__private::ServerFn>::PATH"),
		"{output}"
	);
	insta::assert_snapshot!(output);
}

#[test]
fn http_protocols_are_published() {
	assert!(
		expand(
			TokenStream::new(),
			quote! {
				#[server(protocol = server_fn::Http<server_fn::codec::GetUrl, server_fn::codec::Json>)]
				pub async fn ping() -> Result<(), ServerFnError> {
					Ok(())
				}
			},
			true,
		)
		.is_ok()
	);
}

#[test]
fn wrapped_argument_entries_keep_every_line() {
	let docs = "Lists.\n\n# Arguments\n\n* `limit` - How many to return; the server\n  caps it at \
	            a hundred.\n* `filter` - Which ones.\n`raw` - Without a bullet.\n\nTrailing \
	            prose.\n\n# Examples\n\n* `limit` - Not an argument.";
	let arguments = argument_docs(docs);
	assert_eq!(
		arguments["limit"],
		"How many to return; the server caps it at a hundred."
	);
	assert_eq!(arguments["filter"], "Which ones.");
	assert_eq!(arguments["raw"], "Without a bullet.");
	assert_eq!(arguments.len(), 3, "{arguments:?}");
}

// ---- diagnostics
// -------------------------------------------------------------

#[test]
fn a_missing_server_attribute_points_at_the_function() {
	let (message, underlined) = error("", "pub async fn ping() -> Result<(), ServerFnError> {}");
	assert!(
		message.contains("must be placed above #[server]"),
		"{message}"
	);
	assert_eq!(underlined, "ping");
}

#[test]
fn raw_function_names_are_rejected_with_the_way_out() {
	let (message, underlined) = error(
		"",
		"#[server] pub async fn r#type() -> Result<(), ServerFnError> {}",
	);
	assert!(
		message.contains(r#"publish it as `type` with #[api(name = "type")]"#),
		"{message}"
	);
	assert_eq!(underlined, "r#type");
}

#[test]
fn unknown_options_are_rejected() {
	let (message, underlined) = error(
		"internal, public",
		"#[server] pub async fn ping() -> Result<(), ServerFnError> {}",
	);
	assert!(message.contains("unknown #[api] option"), "{message}");
	assert_eq!(underlined, "public");
}

#[test]
fn names_must_be_strings_and_route_segments() {
	let (message, underlined) = error(
		"name = 1",
		"#[server] pub async fn ping() -> Result<(), ServerFnError> {}",
	);
	assert_eq!(message, "`name` expects a string literal");
	assert_eq!(underlined, "1");

	let (message, underlined) = error(
		r#"name = "todos/list""#,
		"#[server] pub async fn ping() -> Result<(), ServerFnError> {}",
	);
	assert!(message.contains("becomes the route"), "{message}");
	assert_eq!(underlined, r#""todos/list""#);

	let (message, underlined) = error(
		r#"name = "a", name = "b""#,
		"#[server] pub async fn ping() -> Result<(), ServerFnError> {}",
	);
	assert_eq!(message, "`name` is given twice");
	assert_eq!(underlined, r#"name = "b""#);
}

#[test]
fn a_bad_crate_path_is_rejected() {
	let (message, underlined) = error(
		r#"crate = "leptoskit::""#,
		"#[server] pub async fn ping() -> Result<(), ServerFnError> {}",
	);
	assert!(message.contains("the path to `leptoskit_rpc`"), "{message}");
	assert_eq!(underlined, r#""leptoskit::""#);
}

#[test]
fn receivers_and_patterns_are_rejected() {
	let (message, underlined) = error(
		"",
		"#[server] pub async fn ping(&self) -> Result<(), ServerFnError> {}",
	);
	assert_eq!(message, "server functions cannot take `self`");
	assert_eq!(underlined, "&self");

	let (message, underlined) = error(
		"",
		"#[server] pub async fn ping((a, b): (u8, u8)) -> Result<(), ServerFnError> {}",
	);
	assert!(message.contains("destructuring patterns"), "{message}");
	assert_eq!(underlined, "(a, b)");

	let (message, underlined) = error(
		"",
		"#[server] pub async fn ping(_: u8) -> Result<(), ServerFnError> {}",
	);
	assert!(
		message.contains("needs a name for every argument"),
		"{message}"
	);
	assert_eq!(underlined, "_");
}

#[test]
fn websocket_protocols_are_rejected_at_the_codec() {
	let (message, underlined) = error(
		"",
		"#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)] pub async fn chat(input: \
		 BoxedStream<String, ServerFnError>) -> Result<BoxedStream<String, ServerFnError>, \
		 ServerFnError> {}",
	);
	assert!(
		message.contains("cannot publish a `Websocket` server function"),
		"{message}"
	);
	assert_eq!(underlined, "Websocket");
}

#[test]
fn streaming_encodings_are_rejected_wherever_they_appear() {
	let (message, underlined) = error(
		"",
		"#[server(output = StreamingText)] pub async fn tail() -> Result<TextStream, \
		 ServerFnError> {}",
	);
	assert!(message.contains("streaming encodings"), "{message}");
	assert_eq!(underlined, "StreamingText");

	let (message, underlined) = error(
		"",
		"#[server(protocol = Http<codec::Streaming, Json>)] pub async fn upload(data: ByteStream) \
		 -> Result<(), ServerFnError> {}",
	);
	assert!(message.contains("`Streaming`"), "{message}");
	assert_eq!(underlined, "Streaming");

	let (message, underlined) = error(
		"",
		"#[server(input = MultipartFormData)] pub async fn upload(data: MultipartData) -> \
		 Result<(), ServerFnError> {}",
	);
	assert!(message.contains("multipart forms"), "{message}");
	assert_eq!(underlined, "MultipartFormData");
}

#[test]
fn name_value_server_attributes_are_rejected() {
	let (message, underlined) = error(
		"",
		r#"#[server = "x"] pub async fn ping() -> Result<(), ServerFnError> {}"#,
	);
	assert_eq!(message, "expected #[server] or #[server(...)]");
	assert_eq!(underlined, r#"server = "x""#);
}

#[test]
fn positional_attributes_need_an_endpoint() {
	let (message, underlined) = error(
		"",
		r#"#[server(Ping, "/api")] pub async fn ping() -> Result<(), ServerFnError> {}"#,
	);
	assert!(message.contains("needs its fourth argument"), "{message}");
	assert_eq!(underlined, r#"#[server(Ping, "/api")]"#);
}

#[test]
fn flattened_arguments_are_rejected() {
	let (message, underlined) = error(
		"",
		"#[server] pub async fn search(#[server(flatten)] query: Query) -> Result<(), \
		 ServerFnError> {}",
	);
	assert!(message.contains("`#[server(flatten)]`"), "{message}");
	assert_eq!(underlined, "flatten");
}

#[test]
fn malformed_server_arguments_are_reported() {
	let (message, _) = error(
		"",
		"#[server(prefix = 1)] pub async fn ping() -> Result<(), ServerFnError> {}",
	);
	assert!(message.contains("expected string literal"), "{message}");
	let (message, underlined) = error(
		"",
		"#[server(+)] pub async fn ping() -> Result<(), ServerFnError> {}",
	);
	assert_eq!(message, "unexpected token in #[server] arguments");
	assert_eq!(underlined, "+");
}
