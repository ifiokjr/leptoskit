//! `/search?q=&tag=`: notes matching a query, a page of results at a time.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// No field matches a segment, so they all come from the query string:
/// `/search?q=sync&tag=work&tag=ios&page=1`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Search {
	/// The text to look for, empty when the URL leaves it out.
	#[route(default)]
	pub q: String,
	/// Only notes with every one of these tags: `?tag=` repeats.
	pub tag: Vec<String>,
	/// The page of results, from 0. The first page leaves it out of the URL.
	#[route(default)]
	pub page: u32,
}

/// The results so far: the query, its tags, and the page.
pub fn page(route: &Search) -> Element {
	let title = if route.q.is_empty() {
		"Search notes".to_owned()
	} else {
		format!("Notes matching “{}”", route.q)
	};
	let tags = route
		.tag
		.iter()
		.map(|tag| Text(format!("#{tag}")).class("text-sm"));
	View()
		.class("gap-2 p-6")
		.child(Text(title).class("text-2xl font-semibold"))
		.children(tags)
		.child(Text(format!("Page {}", route.page + 1)).class("text-sm"))
}
