use leptos::prelude::*;

#[component]
pub fn Text() -> impl IntoView {
	view! {
		<p>"Plain text, " {name} " and more."</p>
		<p>r#"A "raw" string"#</p>
		<p>Unquoted text keeps its   spacing</p>
		<p>"Unicode: héllo wörld — 日本語 🎉"</p>
		<pre>"line one
  keeps   its
	inner whitespace"</pre>
		<p>"This paragraph is long enough that it cannot fit on one line, so its children are packed " <b>"bold"</b> " onto as few lines as fit, " <i>"italic"</i> " and all."</p>
		<p>"a"{b}"c"</p>
	}
}
