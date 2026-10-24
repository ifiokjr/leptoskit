//! Dialog: a window separated from the page content.

use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_native::components::Heading;
use leptoskit_yoga::Display;

fn title_id(id: &str) -> String {
	format!("{id}-title")
}

/// The dialog container: a modal named by its [`DialogTitle`](fn@DialogTitle).
///
/// A dialog must be named, so the container references `{id}-title` and the
/// renderer's validation rejects a dialog whose title element is missing. The
/// dialog stays mounted while closed, collapsed out of layout, so the title
/// id always resolves; build `open` from state and the same tree opens and
/// closes.
///
/// The container wires the semantics of a modal dialog — role, `aria-modal`,
/// and its name. Focus containment is still pending in the renderer (see the
/// accessibility guide), so a screen reader can leave the dialog today; pair
/// it with an assertive live region when announcing it.
///
/// ```
/// use leptoskit_aria::Dialog;
/// use leptoskit_aria::DialogTitle;
///
/// let dialog = Dialog("discard", false).child(DialogTitle("discard", 2, "Discard draft?"));
///
/// assert_eq!(
/// 	dialog.props.aria().role,
/// 	Some(leptoskit_native::Role::Dialog)
/// );
/// assert_eq!(dialog.props.aria().modal, Some(true));
/// assert_eq!(dialog.props.aria().labelled_by, ["discard-title"]);
/// ```
pub fn Dialog(id: &str, open: bool) -> Element {
	let mut dialog = Element::view()
		.role(Role::Dialog)
		.aria_modal(true)
		.aria_labelled_by([title_id(id)]);

	if !open {
		dialog = dialog.with_style(|style| style.display = Display::None);
	}

	dialog
}

/// The title that names a [`Dialog`](fn@Dialog): a heading at the given
/// level, starting at one.
pub fn DialogTitle(id: &str, level: u8, content: impl Into<String>) -> Element {
	Heading(level, content).id(title_id(id))
}

/// The id a dialog references to describe itself: `{dialog_id}-description`.
///
/// Chain it when the dialog has a description:
///
/// ```
/// use leptoskit_aria::Dialog;
/// use leptoskit_aria::DialogDescription;
/// use leptoskit_aria::DialogTitle;
/// use leptoskit_aria::dialog_description_id;
///
/// let id = "discard";
/// let dialog = Dialog(id, true)
/// 	.aria_described_by([dialog_description_id(id)])
/// 	.child(DialogTitle(id, 2, "Discard draft?"))
/// 	.child(DialogDescription(id, "This cannot be undone."));
/// assert_eq!(dialog.props.aria().described_by, ["discard-description"]);
/// ```
#[must_use]
pub fn dialog_description_id(dialog_id: &str) -> String {
	format!("{dialog_id}-description")
}

/// The description element of a [`Dialog`](fn@Dialog), beyond its name.
///
/// ```
/// use leptoskit_aria::DialogDescription;
///
/// let description = DialogDescription("discard", "This cannot be undone.");
/// assert_eq!(description.props.id.as_deref(), Some("discard-description"));
/// ```
pub fn DialogDescription(id: &str, content: impl Into<String>) -> Element {
	Element::text(content).id(dialog_description_id(id))
}
