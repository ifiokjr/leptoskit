use super::*;

/// `DocumentWrapper` is wrapper struct around [`web_sys::Document`]. We use to
/// allow effortless implementation of `DomQuery` trait on any struct that holds
/// onto a [`web_sys::Document`]
#[derive(Deref, DerefMut)]
pub struct ElementWrapper<'a>(pub &'a Element);

impl ElementWrapper<'_> {
	/// The controls named by the `for` attribute of each `<label>` in
	/// `candidates`.
	///
	/// Text matches that are not labels, labels without `for`, and labels whose
	/// target is missing (or ambiguous) are skipped, so a label query reports
	/// "not found" instead of panicking.
	fn labelled_controls(&self, candidates: Vec<TestElement>) -> Vec<TestElement> {
		candidates
			.into_iter()
			.filter_map(|candidate| {
				let label = candidate.0.dyn_into::<web_sys::HtmlLabelElement>().ok()?;
				let target = label.html_for();
				if target.is_empty() {
					return None;
				}
				self.get_by_id(&target).ok()
			})
			.collect()
	}
}

impl DomQuery for ElementWrapper<'_> {
	fn get_by_text<S: AsRef<str>>(&self, text: S) -> Result<TestElement, TestingLibraryError> {
		get_one_inner(
			self.get_all_by_text(text.as_ref()).as_slice(),
			"by_text",
			text,
		)
	}

	fn get_all_by_text<S: AsRef<str>>(&self, text: S) -> Vec<TestElement> {
		get_all_text_nodes(self.0)
			.find_parents_of_matching_text(text)
			.into_iter()
			.map(TestElement)
			.collect()
	}

	fn get_by_text_contains<S: AsRef<str>>(
		&self,
		text: S,
	) -> Result<TestElement, TestingLibraryError> {
		get_one_inner(
			self.get_all_by_text_contains(text.as_ref()).as_slice(),
			"by_text_contains",
			text,
		)
	}

	fn get_all_by_text_contains<S: AsRef<str>>(&self, text: S) -> Vec<TestElement> {
		get_all_text_nodes(self.0)
			.find_parents_of_containing_text(text)
			.into_iter()
			.map(TestElement)
			.collect()
	}

	fn get_by_id<S: AsRef<str>>(&self, id: S) -> Result<TestElement, TestingLibraryError> {
		get_one_inner(self.get_all_by_id(id.as_ref()).as_slice(), "by_id", id)
	}

	fn get_all_by_id<S: AsRef<str>>(&self, id: S) -> Vec<TestElement> {
		let tag_names = self.0.get_elements_by_tag_name("*");
		let mut list = Vec::new();
		for i in 0..tag_names.length() {
			let item = tag_names.item(i).unwrap();
			if item.id() == id.as_ref()
				&& let Ok(item) = item.dyn_into::<HtmlElement>()
			{
				list.push(item.into());
			}
		}
		list
	}

	fn get_by_id_contains<S: AsRef<str>>(&self, id: S) -> Result<TestElement, TestingLibraryError> {
		get_one_inner(
			self.get_all_by_id_contains(id.as_ref()).as_slice(),
			"by_id_contains",
			id,
		)
	}

	fn get_all_by_id_contains<S: AsRef<str>>(&self, id: S) -> Vec<TestElement> {
		let tag_names = self.0.get_elements_by_tag_name("*");
		let mut list = Vec::new();
		for i in 0..tag_names.length() {
			let item = tag_names.item(i).unwrap();
			if item.id().contains(id.as_ref())
				&& let Ok(item) = item.dyn_into::<HtmlElement>()
			{
				list.push(item.into());
			}
		}
		list
	}

	fn get_by_label<S: AsRef<str>>(&self, text: S) -> Result<TestElement, TestingLibraryError> {
		get_one_inner(
			self.get_all_by_label(text.as_ref()).as_slice(),
			"by_label",
			text,
		)
	}

	fn get_all_by_label<S: AsRef<str>>(&self, text: S) -> Vec<TestElement> {
		self.labelled_controls(self.get_all_by_text(text))
	}

	fn get_by_label_contains<S: AsRef<str>>(
		&self,
		text: S,
	) -> Result<TestElement, TestingLibraryError> {
		get_one_inner(
			self.get_all_by_label_contains(text.as_ref()).as_slice(),
			"by_label",
			text,
		)
	}

	fn get_all_by_label_contains<S: AsRef<str>>(&self, text: S) -> Vec<TestElement> {
		self.labelled_controls(self.get_all_by_text_contains(text))
	}

	fn get_by_display_value<S: AsRef<str>>(
		&self,
		value: S,
	) -> Result<TestElement, TestingLibraryError> {
		get_one_inner(
			self.get_all_by_display_value(value.as_ref()).as_slice(),
			"by_display_value",
			value,
		)
	}

	fn get_all_by_display_value<S: AsRef<str>>(&self, value: S) -> Vec<TestElement> {
		let elements = self
			.0
			.query_selector_all("input, textarea, select")
			.unwrap();
		let mut list = Vec::new();
		for i in 0..elements.length() {
			let item = elements.item(i).unwrap().dyn_into::<HtmlElement>().unwrap();
			if let Some(ref_item) = item.dyn_ref::<web_sys::HtmlTextAreaElement>() {
				if ref_item.value() == value.as_ref() {
					list.push(TestElement(item));
				}
			} else if let Some(ref_item) = item.dyn_ref::<web_sys::HtmlInputElement>() {
				if ref_item.value() == value.as_ref() {
					list.push(TestElement(item));
				}
			} else if let Ok(item) = item.dyn_into::<web_sys::HtmlSelectElement>()
				&& item.value() == value.as_ref()
			{
				list.push(TestElement(item.into()));
			}
		}
		list
	}

	fn get_by_role<S: AsRef<str>>(&self, role: S) -> Result<TestElement, TestingLibraryError> {
		get_one_inner(
			self.get_all_by_role(role.as_ref()).as_slice(),
			"by_role",
			role,
		)
	}

	fn get_all_by_role<S: AsRef<str>>(&self, role: S) -> Vec<TestElement> {
		let selector = format!("[role='{}']", role.as_ref());
		let elements = self.0.query_selector_all(&selector).unwrap();
		let mut list = Vec::new();
		for i in 0..elements.length() {
			list.push(
				elements
					.item(i)
					.unwrap()
					.dyn_into::<HtmlElement>()
					.unwrap()
					.into(),
			);
		}
		list
	}

	fn get_by_placeholder<S: AsRef<str>>(
		&self,
		placeholder: S,
	) -> Result<TestElement, TestingLibraryError> {
		get_one_inner(
			self.get_all_by_placeholder(placeholder.as_ref()).as_slice(),
			"by_placeholder",
			placeholder,
		)
	}

	fn get_all_by_placeholder<S: AsRef<str>>(&self, placeholder: S) -> Vec<TestElement> {
		let elements = self.0.query_selector_all("input, textarea").unwrap();
		let mut list = Vec::new();

		for i in 0..elements.length() {
			let item = elements.item(i).unwrap().dyn_into::<HtmlElement>().unwrap();
			if let Some(ref_item) = item.dyn_ref::<web_sys::HtmlTextAreaElement>() {
				if ref_item.placeholder() == placeholder.as_ref() {
					list.push(TestElement(item));
				}
			} else if let Some(ref_item) = item.dyn_ref::<web_sys::HtmlInputElement>()
				&& ref_item.placeholder() == placeholder.as_ref()
			{
				list.push(TestElement(item));
			}
		}
		list
	}

	fn get_by_placeholder_contains<S: AsRef<str>>(
		&self,
		placeholder: S,
	) -> Result<TestElement, TestingLibraryError> {
		get_one_inner(
			self.get_all_by_placeholder_contains(placeholder.as_ref())
				.as_slice(),
			"by_placeholder",
			placeholder,
		)
	}

	fn get_all_by_placeholder_contains<S: AsRef<str>>(&self, placeholder: S) -> Vec<TestElement> {
		let elements = self.0.query_selector_all("input, textarea").unwrap();
		let mut list = Vec::new();

		for i in 0..elements.length() {
			let item = elements.item(i).unwrap().dyn_into::<HtmlElement>().unwrap();

			if let Some(ref_item) = item.dyn_ref::<web_sys::HtmlTextAreaElement>() {
				if ref_item.placeholder().contains(placeholder.as_ref()) {
					list.push(TestElement(item));
				}
			} else if let Some(ref_item) = item.dyn_ref::<web_sys::HtmlInputElement>()
				&& ref_item.placeholder().contains(placeholder.as_ref())
			{
				list.push(TestElement(item));
			}
		}
		list
	}
}
