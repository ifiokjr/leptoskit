use super::*;

pub(crate) fn get_one_inner<S: AsRef<str>>(
	list: &[TestElement],
	method: &'static str,
	ident: S,
) -> Result<TestElement, TestingLibraryError> {
	if list.len() > 1 {
		Err(TestingLibraryError::more_than_one(
			method,
			ident.as_ref().to_string(),
		))
	} else if list.is_empty() {
		Err(TestingLibraryError::not_found(
			method,
			ident.as_ref().to_string(),
		))
	} else {
		Ok(list.first().cloned().unwrap())
	}
}

fn find_all_text_nodes(node: &Node, text_nodes: &mut Vec<Node>) {
	if node.node_type() == Node::TEXT_NODE {
		text_nodes.push(node.clone());
		return;
	}
	// Elements, documents, and fragments all hold children worth walking.
	let list = node.child_nodes();
	(0..list.length()).for_each(|i| {
		let child = list.get(i).unwrap();
		find_all_text_nodes(&child, text_nodes);
	});
}

pub(crate) fn get_all_text_nodes(node: &Node) -> TextNodes {
	let mut text_nodes = Vec::new();
	find_all_text_nodes(node, &mut text_nodes);
	TextNodes(text_nodes)
}
pub(crate) struct TextNodes(Vec<Node>);
impl TextNodes {
	pub(crate) fn join_text(&self, join_on: &str) -> String {
		self.0
			.iter()
			.map(|node| node.text_content().unwrap())
			.collect::<Vec<String>>()
			.join(join_on)
	}

	/// The distinct HTML elements that directly hold at least one text node.
	///
	/// An element with several text children (`<span>"Count: "{count}</span>`
	/// renders two) appears once, so text queries never report it twice.
	fn text_parents(&self) -> Vec<HtmlElement> {
		let mut parents: Vec<HtmlElement> = Vec::new();
		for node in &self.0 {
			let Some(parent) = node.parent_element() else {
				continue;
			};
			let Ok(parent) = parent.dyn_into::<HtmlElement>() else {
				continue;
			};
			if !parents.iter().any(|seen| seen.is_same_node(Some(&parent))) {
				parents.push(parent);
			}
		}
		parents
	}

	/// Elements whose own text equals `text`. See [`own_text`].
	pub(crate) fn find_parents_of_matching_text<S: AsRef<str>>(&self, text: S) -> Vec<HtmlElement> {
		self.text_parents()
			.into_iter()
			.filter(|element| own_text(element) == text.as_ref())
			.collect()
	}

	/// Elements whose own text contains `text`. See [`own_text`].
	pub(crate) fn find_parents_of_containing_text<S: AsRef<str>>(
		&self,
		text: S,
	) -> Vec<HtmlElement> {
		self.text_parents()
			.into_iter()
			.filter(|element| own_text(element).contains(text.as_ref()))
			.collect()
	}
}

/// The text an element holds directly: its text-node children joined, with
/// whitespace trimmed and collapsed to single spaces.
///
/// Matching against the joined children, not each text node, lets text split
/// across nodes (`"Count: "` then `"0"`) match as `"Count: 0"`. Ignoring
/// descendants' text keeps ancestors such as `<body>` from matching
/// everything inside them.
fn own_text(element: &HtmlElement) -> String {
	let children = element.child_nodes();
	let mut text = String::new();
	for index in 0..children.length() {
		let Some(child) = children.get(index) else {
			continue;
		};
		if child.node_type() == Node::TEXT_NODE {
			text.push_str(&child.text_content().unwrap_or_default());
		}
	}
	text.split_whitespace().collect::<Vec<_>>().join(" ")
}
