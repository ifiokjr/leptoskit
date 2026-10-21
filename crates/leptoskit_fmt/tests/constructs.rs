//! One snapshot per markup construct and edge case.

mod support;

use leptoskit_fmt::AttributeBraces;
use leptoskit_fmt::ClosingTags;
use leptoskit_fmt::FormatConfig;
use leptoskit_fmt::NewlineStyle;
use leptoskit_fmt::Settings;
use support::format;
use support::format_with;

/// Formats `markup` as the tail of a function, and returns the function.
fn view(markup: &str) -> String {
	let output = format(&format!(
		"fn f() -> impl IntoView {{\n\tview! {{ {markup} }}\n}}\n"
	));
	assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
	output.text
}

fn with_markup(markup: FormatConfig, source: &str) -> String {
	let settings = Settings {
		markup,
		..support::SETTINGS.clone()
	};
	format_with(source, &settings).text
}

#[test]
fn single_node_stays_on_one_line_when_it_fits() {
	insta::assert_snapshot!(view("<Counter initial=3/>"), @r"
	fn f() -> impl IntoView {
		view! { <Counter initial=3 /> }
	}
	");
	insta::assert_snapshot!(view(r#"<p class="note">"The note is long enough that the macro no longer fits on its line"</p>"#), @r#"
	fn f() -> impl IntoView {
		view! {
			<p class="note">"The note is long enough that the macro no longer fits on its line"</p>
		}
	}
	"#);
}

#[test]
fn several_nodes_go_on_lines_of_their_own() {
	insta::assert_snapshot!(view(r#"<h1>"Title"</h1> <p>"Body"</p>"#), @r#"
	fn f() -> impl IntoView {
		view! {
			<h1>"Title"</h1>
			<p>"Body"</p>
		}
	}
	"#);
}

#[test]
fn one_attribute_never_wraps() {
	insta::assert_snapshot!(view(r#"<a href="https://example.com/an/extremely/long/address/that/is/longer/than/the/line/allows">"x"</a>"#), @r#"
	fn f() -> impl IntoView {
		view! {
			<a href="https://example.com/an/extremely/long/address/that/is/longer/than/the/line/allows">
				"x"
			</a>
		}
	}
	"#);
}

#[test]
fn several_attributes_wrap_together() {
	insta::assert_snapshot!(view(r#"<input type="text" class="w-full rounded border px-2 py-1" placeholder="Search everything" autofocus/>"#), @r#"
	fn f() -> impl IntoView {
		view! {
			<input
				type="text"
				class="w-full rounded border px-2 py-1"
				placeholder="Search everything"
				autofocus
			/>
		}
	}
	"#);
	insta::assert_snapshot!(view(r#"<div id="a" class="b"><span/></div>"#), @r#"
	fn f() -> impl IntoView {
		view! {
			<div id="a" class="b">
				<span />
			</div>
		}
	}
	"#);
}

#[test]
fn event_handlers_keep_their_closures() {
	insta::assert_snapshot!(view("<button on:click=move |_| { let next = count.get() + 1; set_count.set(next); }>\"+\"</button>"), @r#"
	fn f() -> impl IntoView {
		view! {
			<button on:click=move |_| {
				let next = count.get() + 1;
				set_count.set(next);
			}>
				"+"
			</button>
		}
	}
	"#);
	insta::assert_snapshot!(view("<input on:input=move |ev| set_value.set(event_target_value(&ev)) on:blur=move |_| validate() />"), @r"
	fn f() -> impl IntoView {
		view! {
			<input
				on:input=move |ev| set_value.set(event_target_value(&ev))
				on:blur=move |_| validate()
			/>
		}
	}
	");
}

#[test]
fn textual_children_pack_like_a_paragraph() {
	insta::assert_snapshot!(view(r#"<p>"Count: "{count}" of "{total}</p>"#), @r#"
	fn f() -> impl IntoView {
		view! { <p>"Count: "{count}" of "{total}</p> }
	}
	"#);
	insta::assert_snapshot!(view(r#"<p>"One sentence that is quite long, " <b>"then bold"</b> ", then more words that keep going " <i>"and italic"</i> " to the end."</p>"#), @r#"
	fn f() -> impl IntoView {
		view! {
			<p>
				"One sentence that is quite long, " <b>"then bold"</b>
				", then more words that keep going " <i>"and italic"</i> " to the end."
			</p>
		}
	}
	"#);
}

#[test]
fn element_children_stack() {
	insta::assert_snapshot!(view("<ul><li>\"a\"</li><li>\"b\"</li></ul>"), @r#"
	fn f() -> impl IntoView {
		view! {
			<ul>
				<li>"a"</li>
				<li>"b"</li>
			</ul>
		}
	}
	"#);
}

#[test]
fn literals_and_unquoted_text_are_copied_exactly() {
	insta::assert_snapshot!(view(r##"<p>r#"raw "quoted" text"#</p> <p>Unquoted   text</p> <p>'c'</p> <p>b"bytes"</p>"##), @r##"
	fn f() -> impl IntoView {
		view! {
			<p>r#"raw "quoted" text"#</p>
			<p>Unquoted   text</p>
			<p>'c'</p>
			<p>b"bytes"</p>
		}
	}
	"##);
}

#[test]
fn multi_line_strings_keep_their_lines() {
	let source =
		"fn f() -> impl IntoView {\n\tview! {\n<pre title=\"a\nb\">\"one\n  two\"</pre>\n\t}\n}\n";
	insta::assert_snapshot!(format(source).text, @r#"
	fn f() -> impl IntoView {
		view! {
			<pre title="a
	b">
				"one
	  two"
			</pre>
		}
	}
	"#);
}

#[test]
fn fragments_lay_out_like_elements() {
	insta::assert_snapshot!(view("<><A/><B/></>"), @r"
	fn f() -> impl IntoView {
		view! {
			<>
				<A />
				<B />
			</>
		}
	}
	");
	insta::assert_snapshot!(view("<>\"text\"</>"), @r#"
	fn f() -> impl IntoView {
		view! { <>"text"</> }
	}
	"#);
}

#[test]
fn components_with_props_generics_and_bindings() {
	insta::assert_snapshot!(view("<For each=move || rows.get() key=|row| row.id let:row><Row row=row/></For>"), @r"
	fn f() -> impl IntoView {
		view! {
			<For each=move || rows.get() key=|row| row.id let:row>
				<Row row=row />
			</For>
		}
	}
	");
	insta::assert_snapshot!(view("<Select<Option> options=options on_change=set_choice/>"), @r"
	fn f() -> impl IntoView {
		view! { <Select<Option> options=options on_change=set_choice /> }
	}
	");
	insta::assert_snapshot!(view("<Await future=load() let(data)><p>{data}</p></Await>"), @r"
	fn f() -> impl IntoView {
		view! {
			<Await future=load() let(data)>
				<p>{data}</p>
			</Await>
		}
	}
	");
}

#[test]
fn spreads_keep_the_braces_before_them() {
	insta::assert_snapshot!(view("<div class={name} {..} />"), @r"
	fn f() -> impl IntoView {
		view! { <div class={name} {..} /> }
	}
	");
	insta::assert_snapshot!(view("<div class={name.get()} {..attrs} id={x}/>"), @r"
	fn f() -> impl IntoView {
		view! { <div class=name.get() {..attrs} id=x /> }
	}
	");
}

#[test]
fn closures_and_blocks_as_children() {
	insta::assert_snapshot!(view("<div>{move || if ready.get() { \"ready\" } else { \"waiting\" }}</div>"), @r#"
	fn f() -> impl IntoView {
		view! { <div>{move || if ready.get() { "ready" } else { "waiting" }}</div> }
	}
	"#);
	insta::assert_snapshot!(view("<div>{ let a = 1; let b = 2; a + b }</div>"), @r"
	fn f() -> impl IntoView {
		view! {
			<div>
				{
					let a = 1;
					let b = 2;
					a + b
				}
			</div>
		}
	}
	");
}

#[test]
fn comments_between_attributes_and_children() {
	let source = "fn f() -> impl IntoView {\n\tview! {\n<div\n// the id\nid=\"a\" /* why */ \
	              class=\"b\"\n>\n\"text\" // trailing text\n<span/>\n\n\n// \
	              last\n</div>\n\t}\n}\n";
	insta::assert_snapshot!(format(source).text, @r#"
	fn f() -> impl IntoView {
		view! {
			<div
				// the id
				id="a" /* why */
				class="b"
			>
				"text" // trailing text
				<span />

				// last
			</div>
		}
	}
	"#);
}

#[test]
fn nested_macros_lay_out_where_they_land() {
	insta::assert_snapshot!(view("<Outer>{move || view! { <Middle>{items.iter().map(|item| view! { <Inner value=item.clone()/> }).collect_view()}</Middle> }}</Outer>"), @r"
	fn f() -> impl IntoView {
		view! {
			<Outer>
				{move || {
					view! {
						<Middle>
							{items
								.iter()
								.map(|item| view! { <Inner value=item.clone() /> })
								.collect_view()}
						</Middle>
					}
				}}
			</Outer>
		}
	}
	");
	insta::assert_snapshot!(view("<Show when=move || open.get() fallback=|| view! { <Closed/> }><Open/></Show>"), @r"
	fn f() -> impl IntoView {
		view! {
			<Show when=move || open.get() fallback=|| view! { <Closed /> }>
				<Open />
			</Show>
		}
	}
	");
}

#[test]
fn unicode_counts_by_display_width() {
	insta::assert_snapshot!(view(r#"<p title="日本語のタイトル">"日本語のテキストは幅が二倍なので、この段落は行に収まりません。"</p>"#), @r#"
	fn f() -> impl IntoView {
		view! {
			<p title="日本語のタイトル">
				"日本語のテキストは幅が二倍なので、この段落は行に収まりません。"
			</p>
		}
	}
	"#);
	insta::assert_snapshot!(view(r#"<p>"emoji 🎉 and accents éàü"</p>"#), @r#"
	fn f() -> impl IntoView {
		view! { <p>"emoji 🎉 and accents éàü"</p> }
	}
	"#);
}

#[test]
fn crlf_and_byte_order_mark_are_kept() {
	let source = "\u{feff}fn f() -> impl IntoView {\r\n\tview! {   <a/>   }\r\n}\r\n";
	let mut settings = support::SETTINGS.clone();
	settings.layout.newline_style = NewlineStyle::Auto;
	let output = format_with(source, &settings);
	assert_eq!(
		output.text,
		"\u{feff}fn f() -> impl IntoView {\r\n\tview! { <a /> }\r\n}\r\n"
	);
}

#[test]
fn blank_lines_collapse_to_one() {
	let source = "fn f() -> impl IntoView {\n\tview! {\n<A/>\n\n\n\n<B/>\n\t}\n}\n";
	insta::assert_snapshot!(format(source).text, @r"
	fn f() -> impl IntoView {
		view! {
			<A />

			<B />
		}
	}
	");
}

#[test]
fn scoped_class_prefix() {
	insta::assert_snapshot!(view("class = STYLE, <div/>"), @r"
	fn f() -> impl IntoView {
		view! {
			class=STYLE,
			<div />
		}
	}
	");
}

#[test]
fn delimiters_are_kept() {
	let source = "fn f() {\n\tlet a = view!(<a/>);\n\tlet b = view![<b/>];\n\tlet c = view! \
	              {};\n\tlet d = view!(<div><a/></div>).into_any();\n}\n";
	insta::assert_snapshot!(format(source).text, @r"
	fn f() {
		let a = view!(<a />);
		let b = view![<b />];
		let c = view! {};
		let d = view!(
			<div>
				<a />
			</div>
		)
		.into_any();
	}
	");
}

#[test]
fn macros_in_statement_and_argument_positions() {
	let source =
		"fn f() {\n\tmount_to_body(|| view! { <App/> });\n\tlet rows = data.iter().map(|row| \
		 view! { <tr><td>{row.name.clone()}</td><td>{row.value}</td></tr> \
		 }).collect_view();\n\tmatch state { State::A => view! { <A/> }.into_any(), State::B => \
		 view! { <div class=\"long-class-name\"><B/></div> }.into_any() }\n}\n";
	insta::assert_snapshot!(format(source).text, @r#"
	fn f() {
		mount_to_body(|| view! { <App /> });
		let rows = data
			.iter()
			.map(|row| {
				view! {
					<tr>
						<td>{row.name.clone()}</td>
						<td>{row.value}</td>
					</tr>
				}
			})
			.collect_view();
		match state {
			State::A => view! { <A /> }.into_any(),
			State::B => view! {
				<div class="long-class-name">
					<B />
				</div>
			}
			.into_any(),
		}
	}
	"#);
}

#[test]
fn rustfmt_skip_is_respected() {
	let source = "#[rustfmt::skip]\nfn f() -> impl IntoView {\n\tview! {     <a/>   }\n}\n";
	assert_eq!(format(source).text, source);
}

#[test]
fn attribute_brace_styles() {
	let source = "fn f() -> impl IntoView {\n\tview! { <a href=\"x\" title={title} id=name \
	              class={a > b}/> }\n}\n";
	let style = |attribute_braces| {
		with_markup(
			FormatConfig {
				attribute_braces,
				..FormatConfig::default()
			},
			source,
		)
	};
	insta::assert_snapshot!(style(AttributeBraces::WhenRequired), @r#"
	fn f() -> impl IntoView {
		view! { <a href="x" title=title id=name class={a > b} /> }
	}
	"#);
	insta::assert_snapshot!(style(AttributeBraces::Always), @r#"
	fn f() -> impl IntoView {
		view! { <a href={"x"} title={title} id={name} class={a > b} /> }
	}
	"#);
	insta::assert_snapshot!(style(AttributeBraces::AlwaysUnlessLiteral), @r#"
	fn f() -> impl IntoView {
		view! { <a href="x" title={title} id={name} class={a > b} /> }
	}
	"#);
	insta::assert_snapshot!(style(AttributeBraces::Preserve), @r#"
	fn f() -> impl IntoView {
		view! { <a href="x" title={title} id=name class={a > b} /> }
	}
	"#);
}

#[test]
fn parenthesized_values_keep_their_braces_as_written() {
	// leptoskit spreads `name=(a, b)` into two arguments but passes
	// `name={(a, b)}` as one tuple, so no style may add or remove those braces.
	let source = "fn f() -> impl IntoView {\n\tview! { <View margin=(1, 2) padding={(3, 4)} \
	              flex={(1)} /> }\n}\n";
	for attribute_braces in [
		AttributeBraces::WhenRequired,
		AttributeBraces::Always,
		AttributeBraces::AlwaysUnlessLiteral,
		AttributeBraces::Preserve,
	] {
		let markup = FormatConfig {
			attribute_braces,
			..FormatConfig::default()
		};
		assert_eq!(with_markup(markup, source), source, "{attribute_braces:?}");
	}
}

#[test]
fn closing_tag_styles() {
	let source = "fn f() -> impl IntoView {\n\tview! { \
	              <div><span></span><b/><br></br><Component/></div> }\n}\n";
	let style = |closing_tags| {
		with_markup(
			FormatConfig {
				closing_tags,
				..FormatConfig::default()
			},
			source,
		)
	};
	insta::assert_snapshot!(style(ClosingTags::SelfClosing), @r"
	fn f() -> impl IntoView {
		view! {
			<div>
				<span />
				<b />
				<br />
				<Component />
			</div>
		}
	}
	");
	insta::assert_snapshot!(style(ClosingTags::Explicit), @r"
	fn f() -> impl IntoView {
		view! {
			<div>
				<span></span>
				<b></b>
				<br />
				<Component></Component>
			</div>
		}
	}
	");
	insta::assert_snapshot!(style(ClosingTags::Preserve), @r"
	fn f() -> impl IntoView {
		view! {
			<div>
				<span></span>
				<b />
				<br></br>
				<Component />
			</div>
		}
	}
	");
}

#[test]
fn macro_names_are_configurable() {
	let source = "fn f() {\n\thtml! {   <a/>   };\n\tview! {   <b/>   };\n}\n";
	let markup = FormatConfig {
		macros: vec!["html".to_owned()],
		..FormatConfig::default()
	};
	insta::assert_snapshot!(with_markup(markup, source), @r"
	fn f() {
		html! { <a /> };
		view! {   <b/>   };
	}
	");
}

#[test]
fn layout_follows_rustfmt_settings() {
	let source = "fn f() -> impl IntoView {\n    view! { <div class=\"container\"><p \
	              class=\"x\">\"Some text that is long enough to wrap\"</p></div> }\n}\n";
	let narrow = support::fixtures_dir().join("narrow").join("narrow.rs");
	let settings = Settings::for_path(&narrow).expect("settings");
	assert_eq!(settings.layout.max_width, 60);
	insta::assert_snapshot!(format_with(source, &settings).text, @r#"
	fn f() -> impl IntoView {
	  view! {
	    <div class="container">
	      <p class="x">
	        "Some text that is long enough to wrap"
	      </p>
	    </div>
	  }
	}
	"#);
}
