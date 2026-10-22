//! ARIA-shaped accessibility properties shared by every platform backend.
//!
//! The vocabulary is WAI-ARIA: roles, states, properties, and relationships
//! exactly as the web defines them. React Native settled on the same `aria-*`
//! names for its cross-platform surface, and GTK 4's accessible API mirrors
//! them almost one to one, so the shared tree carries ARIA and each backend
//! maps it onto its platform: the web renderer emits real attributes and
//! semantic elements, iOS sets `UIAccessibility` traits and values, macOS sets
//! `NSAccessibility` roles, Android fills `AccessibilityNodeInfo`, Windows
//! annotates UI Automation properties, and GTK sets accessible roles, states,
//! and properties.
//!
//! Naming follows the accname computation: an explicit
//! [`label`](Aria::label) or a [`labelled_by`](Aria::labelled_by) reference
//! wins over visible text, and [`Element::resolved`] folds referenced names
//! into `label` for native backends, which have no relationship APIs.

use std::collections::BTreeMap;
use std::fmt;

use crate::props::NativeKind;

/// A WAI-ARIA 1.2 concrete role.
///
/// Widget kinds carry implicit roles (a `Button` is a
/// [`Button`](Role::Button)); an explicit role overrides or extends them, the
/// same way the `role` attribute does in HTML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
	/// A message with important, time-sensitive information.
	Alert,
	/// An alert dialog that requires user response.
	AlertDialog,
	/// A region announcing screen-reader shortcuts and commands.
	Application,
	/// A self-contained composition in a document stream.
	Article,
	/// The page's prime heading area.
	Banner,
	/// A quotation from another source.
	Blockquote,
	/// An activation control.
	Button,
	/// The caption of a table or figure.
	Caption,
	/// A cell in a table row.
	Cell,
	/// A boolean or mixed-state choice.
	Checkbox,
	/// A fragment of computer code.
	Code,
	/// A column header cell in a table.
	ColumnHeader,
	/// A composite of a text input and a popup list.
	ComboBox,
	/// A supporting section, peripheral to the main content.
	Complementary,
	/// The page's footer area.
	ContentInfo,
	/// The meaning of a term.
	Definition,
	/// Content marked for removal.
	Deletion,
	/// A dialog: a window separated from the page content.
	Dialog,
	/// An element that declares its own keyboard focus handling.
	Document,
	/// Stress emphasis.
	Emphasis,
	/// A scrollable list of articles.
	Feed,
	/// A figure with an optional caption.
	Figure,
	/// A collection of form-associated elements.
	Form,
	/// A nameless container with no semantic meaning of its own.
	Generic,
	/// A composite widget with tabular data and interactions.
	Grid,
	/// A cell in a grid or treegrid.
	GridCell,
	/// A set of user-interface controls not part of a larger widget.
	Group,
	/// A heading for a section of the page.
	Heading,
	/// An image or graphic.
	Img,
	/// Content marked as an addition.
	Insertion,
	/// An activation control that navigates to a resource.
	Link,
	/// A collection of list items.
	List,
	/// A widget letting the user select from a list of options.
	ListBox,
	/// A single item in a list or directory.
	ListItem,
	/// A region where new information is appended in log order.
	Log,
	/// The main content of the page.
	Main,
	/// A scrolling region where information changes dynamically.
	Marquee,
	/// Mathematical content.
	Math,
	/// A menu of actions or commands.
	Menu,
	/// A row of menus.
	MenuBar,
	/// An option in a menu.
	MenuItem,
	/// A menu item with a checkbox state.
	MenuItemCheckbox,
	/// A menu item with a radio state.
	MenuItemRadio,
	/// A scalar measurement in a known range.
	Meter,
	/// A navigation region of links.
	Navigation,
	/// No semantic meaning; the same as [`Role::Presentation`].
	None,
	/// A parenthetic or ancillary remark.
	Note,
	/// A selectable choice in a list box.
	Option,
	/// A block of prose.
	Paragraph,
	/// Semantic meaning removed for assistive technology.
	Presentation,
	/// An element showing progress toward completion.
	ProgressBar,
	/// A radio option; siblings in a group are exclusive.
	Radio,
	/// A set of exclusive radio options.
	RadioGroup,
	/// A perceivable, named section of the page.
	Region,
	/// A row of cells in a table.
	Row,
	/// A set of rows in a table.
	RowGroup,
	/// A row header cell in a table.
	RowHeader,
	/// A scroll indicator for a container's content.
	ScrollBar,
	/// A landmark for search controls.
	Search,
	/// A text input for entering search terms.
	SearchBox,
	/// A divider between sections or groups.
	Separator,
	/// A numeric input over a range.
	Slider,
	/// A numeric input over a range, with a text form.
	SpinButton,
	/// An advisory live region of text.
	Status,
	/// Importance, seriousness, or urgency.
	Strong,
	/// Text rendered below the baseline.
	Subscript,
	/// Text rendered above the baseline.
	Superscript,
	/// An on/off choice.
	Switch,
	/// A tab control inside a tab list.
	Tab,
	/// A section of tabular data.
	Table,
	/// A set of tabs.
	TabList,
	/// The content area of one tab.
	TabPanel,
	/// A word or phrase with an optional definition.
	Term,
	/// An editable text field.
	TextBox,
	/// A specific point in time.
	Time,
	/// A count of elapsed time, updating dynamically.
	Timer,
	/// A set of buttons or controls.
	Toolbar,
	/// A transient hover or focus description.
	Tooltip,
	/// A hierarchy of options, collapsible and selectable.
	Tree,
	/// A tree with tabular columns.
	TreeGrid,
	/// An option in a tree.
	TreeItem,
}

impl Role {
	/// Every role, in spec order.
	pub const ALL: &'static [Self] = &[
		Self::Alert,
		Self::AlertDialog,
		Self::Application,
		Self::Article,
		Self::Banner,
		Self::Blockquote,
		Self::Button,
		Self::Caption,
		Self::Cell,
		Self::Checkbox,
		Self::Code,
		Self::ColumnHeader,
		Self::ComboBox,
		Self::Complementary,
		Self::ContentInfo,
		Self::Definition,
		Self::Deletion,
		Self::Dialog,
		Self::Document,
		Self::Emphasis,
		Self::Feed,
		Self::Figure,
		Self::Form,
		Self::Generic,
		Self::Grid,
		Self::GridCell,
		Self::Group,
		Self::Heading,
		Self::Img,
		Self::Insertion,
		Self::Link,
		Self::List,
		Self::ListBox,
		Self::ListItem,
		Self::Log,
		Self::Main,
		Self::Marquee,
		Self::Math,
		Self::Menu,
		Self::MenuBar,
		Self::MenuItem,
		Self::MenuItemCheckbox,
		Self::MenuItemRadio,
		Self::Meter,
		Self::Navigation,
		Self::None,
		Self::Note,
		Self::Option,
		Self::Paragraph,
		Self::Presentation,
		Self::ProgressBar,
		Self::Radio,
		Self::RadioGroup,
		Self::Region,
		Self::Row,
		Self::RowGroup,
		Self::RowHeader,
		Self::ScrollBar,
		Self::Search,
		Self::SearchBox,
		Self::Separator,
		Self::Slider,
		Self::SpinButton,
		Self::Status,
		Self::Strong,
		Self::Subscript,
		Self::Superscript,
		Self::Switch,
		Self::Tab,
		Self::Table,
		Self::TabList,
		Self::TabPanel,
		Self::Term,
		Self::TextBox,
		Self::Time,
		Self::Timer,
		Self::Toolbar,
		Self::Tooltip,
		Self::Tree,
		Self::TreeGrid,
		Self::TreeItem,
	];

	/// The ARIA role name, shared by the wire format and the `role` attribute.
	#[must_use]
	pub const fn wire_name(self) -> &'static str {
		match self {
			Self::Alert => "alert",
			Self::AlertDialog => "alertdialog",
			Self::Application => "application",
			Self::Article => "article",
			Self::Banner => "banner",
			Self::Blockquote => "blockquote",
			Self::Button => "button",
			Self::Caption => "caption",
			Self::Cell => "cell",
			Self::Checkbox => "checkbox",
			Self::Code => "code",
			Self::ColumnHeader => "columnheader",
			Self::ComboBox => "combobox",
			Self::Complementary => "complementary",
			Self::ContentInfo => "contentinfo",
			Self::Definition => "definition",
			Self::Deletion => "deletion",
			Self::Dialog => "dialog",
			Self::Document => "document",
			Self::Emphasis => "emphasis",
			Self::Feed => "feed",
			Self::Figure => "figure",
			Self::Form => "form",
			Self::Generic => "generic",
			Self::Grid => "grid",
			Self::GridCell => "gridcell",
			Self::Group => "group",
			Self::Heading => "heading",
			Self::Img => "img",
			Self::Insertion => "insertion",
			Self::Link => "link",
			Self::List => "list",
			Self::ListBox => "listbox",
			Self::ListItem => "listitem",
			Self::Log => "log",
			Self::Main => "main",
			Self::Marquee => "marquee",
			Self::Math => "math",
			Self::Menu => "menu",
			Self::MenuBar => "menubar",
			Self::MenuItem => "menuitem",
			Self::MenuItemCheckbox => "menuitemcheckbox",
			Self::MenuItemRadio => "menuitemradio",
			Self::Meter => "meter",
			Self::Navigation => "navigation",
			Self::None => "none",
			Self::Note => "note",
			Self::Option => "option",
			Self::Paragraph => "paragraph",
			Self::Presentation => "presentation",
			Self::ProgressBar => "progressbar",
			Self::Radio => "radio",
			Self::RadioGroup => "radiogroup",
			Self::Region => "region",
			Self::Row => "row",
			Self::RowGroup => "rowgroup",
			Self::RowHeader => "rowheader",
			Self::ScrollBar => "scrollbar",
			Self::Search => "search",
			Self::SearchBox => "searchbox",
			Self::Separator => "separator",
			Self::Slider => "slider",
			Self::SpinButton => "spinbutton",
			Self::Status => "status",
			Self::Strong => "strong",
			Self::Subscript => "subscript",
			Self::Superscript => "superscript",
			Self::Switch => "switch",
			Self::Tab => "tab",
			Self::Table => "table",
			Self::TabList => "tablist",
			Self::TabPanel => "tabpanel",
			Self::Term => "term",
			Self::TextBox => "textbox",
			Self::Time => "time",
			Self::Timer => "timer",
			Self::Toolbar => "toolbar",
			Self::Tooltip => "tooltip",
			Self::Tree => "tree",
			Self::TreeGrid => "treegrid",
			Self::TreeItem => "treeitem",
		}
	}

	/// The role with this ARIA name, if it exists.
	#[must_use]
	pub fn from_wire(name: &str) -> Option<Self> {
		Self::ALL
			.iter()
			.copied()
			.find(|role| role.wire_name() == name)
	}

	/// The role a widget kind implies without an explicit override.
	#[must_use]
	pub const fn implicit(kind: NativeKind) -> Option<Self> {
		match kind {
			NativeKind::View
			| NativeKind::Text
			| NativeKind::TextRun
			| NativeKind::ScrollView
			| NativeKind::Custom(_) => None,
			NativeKind::Image => Some(Self::Img),
			NativeKind::TextInput | NativeKind::PasswordInput => Some(Self::TextBox),
			NativeKind::Button | NativeKind::Pressable => Some(Self::Button),
			NativeKind::Switch => Some(Self::Switch),
			NativeKind::Checkbox => Some(Self::Checkbox),
			NativeKind::Radio => Some(Self::Radio),
			NativeKind::Slider => Some(Self::Slider),
			NativeKind::LinearProgressIndicator | NativeKind::ActivityIndicator => {
				Some(Self::ProgressBar)
			}
		}
	}

	/// Whether assistive technology activates this role like a control.
	///
	/// The web renderer gives such elements keyboard focus and Enter or Space
	/// activation; native backends make them accessibility elements with an
	/// activation action.
	#[must_use]
	pub const fn is_interactive(self) -> bool {
		matches!(
			self,
			Self::Button
				| Self::Checkbox
				| Self::ComboBox
				| Self::GridCell
				| Self::Link
				| Self::MenuItem
				| Self::MenuItemCheckbox
				| Self::MenuItemRadio
				| Self::Option
				| Self::Radio
				| Self::ScrollBar
				| Self::SearchBox
				| Self::Slider
				| Self::SpinButton
				| Self::Switch
				| Self::Tab
				| Self::TextBox
				| Self::TreeItem
		)
	}

	/// Whether this role exposes a numeric range.
	#[must_use]
	pub const fn is_range(self) -> bool {
		matches!(
			self,
			Self::Meter | Self::ProgressBar | Self::ScrollBar | Self::Slider | Self::SpinButton
		)
	}

	/// Whether this role removes semantics entirely.
	#[must_use]
	pub const fn is_presentational(self) -> bool {
		matches!(self, Self::None | Self::Presentation)
	}

	/// Whether this role is a page landmark.
	#[must_use]
	pub const fn is_landmark(self) -> bool {
		matches!(
			self,
			Self::Banner
				| Self::Complementary
				| Self::ContentInfo
				| Self::Form
				| Self::Main
				| Self::Navigation
				| Self::Region
				| Self::Search
		)
	}

	/// The live politeness this role implies without `aria-live`, if any.
	#[must_use]
	pub const fn implicit_live(self) -> Option<Live> {
		match self {
			Self::Alert => Some(Live::Assertive),
			Self::Log | Self::Status => Some(Live::Polite),
			_ => None,
		}
	}

	/// Whether this role implies announcing the region as a whole.
	#[must_use]
	pub const fn implicit_atomic(self) -> bool {
		matches!(self, Self::Alert | Self::Status)
	}
}

impl fmt::Display for Role {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.wire_name())
	}
}

#[cfg(feature = "serde")]
impl serde::Serialize for Role {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		serializer.serialize_str(self.wire_name())
	}
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Role {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct Visitor;

		impl serde::de::Visitor<'_> for Visitor {
			type Value = Role;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				f.write_str("an ARIA role name")
			}

			fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
			where
				E: serde::de::Error,
			{
				Role::from_wire(value).ok_or_else(|| E::unknown_variant(value, &[]))
			}
		}

		deserializer.deserialize_str(Visitor)
	}
}

/// A three-state value for `aria-checked` and `aria-pressed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "lowercase")
)]
pub enum Tristate {
	/// The state is off.
	#[default]
	False,
	/// The state is on.
	True,
	/// The state applies to some, but not all, of the affected items.
	Mixed,
}

impl Tristate {
	/// The `aria-checked` or `aria-pressed` attribute value.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::False => "false",
			Self::True => "true",
			Self::Mixed => "mixed",
		}
	}
}

impl From<bool> for Tristate {
	fn from(value: bool) -> Self {
		if value { Self::True } else { Self::False }
	}
}

/// When a live region should be announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "lowercase")
)]
pub enum Live {
	/// Not a live region.
	#[default]
	Off,
	/// Announce changes when the user is idle.
	Polite,
	/// Announce changes immediately, interrupting current speech.
	Assertive,
}

impl Live {
	/// The `aria-live` attribute value.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Off => "off",
			Self::Polite => "polite",
			Self::Assertive => "assertive",
		}
	}
}

/// What `aria-current` marks the element as being.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "lowercase")
)]
pub enum Current {
	/// Not current.
	#[default]
	False,
	/// The current item, without a specific kind.
	True,
	/// The current page within a set of pages.
	Page,
	/// The current step within a process.
	Step,
	/// The current location or environment.
	Location,
	/// The current date within a collection of dates.
	Date,
	/// The current time within a set of times.
	Time,
}

impl Current {
	/// The `aria-current` attribute value.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::False => "false",
			Self::True => "true",
			Self::Page => "page",
			Self::Step => "step",
			Self::Location => "location",
			Self::Date => "date",
			Self::Time => "time",
		}
	}
}

/// Why `aria-invalid` marks a value invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "lowercase")
)]
pub enum Invalid {
	/// The value is valid.
	#[default]
	False,
	/// The value is invalid, without a specific kind.
	True,
	/// A grammatical error.
	Grammar,
	/// A spelling error.
	Spelling,
}

impl Invalid {
	/// The `aria-invalid` attribute value.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::False => "false",
			Self::True => "true",
			Self::Grammar => "grammar",
			Self::Spelling => "spelling",
		}
	}
}

impl From<bool> for Invalid {
	fn from(value: bool) -> Self {
		if value { Self::True } else { Self::False }
	}
}

/// What `aria-haspopup` opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "lowercase")
)]
pub enum HasPopup {
	/// No popup.
	#[default]
	False,
	/// A popup of an unspecified kind.
	True,
	/// A menu popup.
	Menu,
	/// A list box popup.
	ListBox,
	/// A tree popup.
	Tree,
	/// A grid popup.
	Grid,
	/// A dialog popup.
	Dialog,
}

impl HasPopup {
	/// The `aria-haspopup` attribute value.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::False => "false",
			Self::True => "true",
			Self::Menu => "menu",
			Self::ListBox => "listbox",
			Self::Tree => "tree",
			Self::Grid => "grid",
			Self::Dialog => "dialog",
		}
	}
}

impl From<bool> for HasPopup {
	fn from(value: bool) -> Self {
		if value { Self::True } else { Self::False }
	}
}

/// The axis `aria-orientation` lays a widget out on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "lowercase")
)]
pub enum Orientation {
	/// Horizontal.
	#[default]
	Horizontal,
	/// Vertical.
	Vertical,
}

impl Orientation {
	/// The `aria-orientation` attribute value.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Horizontal => "horizontal",
			Self::Vertical => "vertical",
		}
	}
}

/// The ARIA attributes with typed [`Aria`] fields; `aria_attr` cannot repeat
/// them.
pub(crate) const TYPED_ATTRIBUTES: &[&str] = &[
	"aria-label",
	"aria-labelledby",
	"aria-description",
	"aria-describedby",
	"aria-roledescription",
	"aria-level",
	"aria-posinset",
	"aria-setsize",
	"aria-orientation",
	"aria-hidden",
	"aria-busy",
	"aria-checked",
	"aria-pressed",
	"aria-selected",
	"aria-expanded",
	"aria-current",
	"aria-modal",
	"aria-required",
	"aria-invalid",
	"aria-readonly",
	"aria-multiline",
	"aria-haspopup",
	"aria-valuenow",
	"aria-valuemin",
	"aria-valuemax",
	"aria-valuetext",
	"aria-live",
	"aria-atomic",
	"aria-controls",
];

/// Whether `aria_attr` may carry this attribute name.
///
/// Names must be lowercase `aria-*` attributes without a typed field.
#[must_use]
pub fn accepts_attribute(name: &str) -> bool {
	let Some(rest) = name.strip_prefix("aria-") else {
		return false;
	};

	!rest.is_empty()
		&& name
			.bytes()
			.all(|byte| byte.is_ascii_lowercase() || byte == b'-')
		&& !TYPED_ATTRIBUTES.contains(&name)
}

/// The ARIA accessibility properties of one element.
///
/// Every field mirrors the same-named WAI-ARIA attribute; `None` and empty
/// collections mean "unset" and serialize away. The web renderer emits each as
/// a DOM attribute, and each native backend maps it onto its platform's
/// accessibility API. [`Element::resolved`](crate::Element::resolved) folds
/// relationship targets and radio set positions into concrete fields for
/// backends.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(default)
)]
pub struct Aria {
	/// The ids of elements naming this one (`aria-labelledby`).
	///
	/// References win over `label`, following the accname computation. The
	/// shared resolver folds them into `label` for native backends; the web
	/// renderer keeps the id references so browsers compute the name live.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_labelled_by", skip_serializing_if = "Vec::is_empty")
	)]
	pub labelled_by: Vec<String>,
	/// A description beyond the name (`aria-description`).
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_description", skip_serializing_if = "Option::is_none")
	)]
	pub description: Option<String>,
	/// The ids of elements describing this one (`aria-describedby`), folded
	/// into `description` for native backends.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_described_by", skip_serializing_if = "Vec::is_empty")
	)]
	pub described_by: Vec<String>,
	/// A human-readable role label (`aria-roledescription`), such as "slide"
	/// for a region.
	#[cfg_attr(
		feature = "serde",
		serde(
			rename = "aria_role_description",
			skip_serializing_if = "Option::is_none"
		)
	)]
	pub role_description: Option<String>,
	/// The element's role (`role`), overriding the widget kind's implicit role.
	#[cfg_attr(
		feature = "serde",
		serde(
			rename = "role",
			skip_serializing_if = "Option::is_none",
			deserialize_with = "lenient_role"
		)
	)]
	pub role: Option<Role>,
	/// The heading, list, or tree depth (`aria-level`), starting at one.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_level", skip_serializing_if = "Option::is_none")
	)]
	pub level: Option<u32>,
	/// The one-based position in a set (`aria-posinset`); radio groups fill
	/// this in automatically.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_pos_in_set", skip_serializing_if = "Option::is_none")
	)]
	pub pos_in_set: Option<u32>,
	/// The number of items in the set (`aria-setsize`); radio groups fill this
	/// in automatically.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_set_size", skip_serializing_if = "Option::is_none")
	)]
	pub set_size: Option<u32>,
	/// Whether a widget's axis is horizontal or vertical
	/// (`aria-orientation`).
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_orientation", skip_serializing_if = "Option::is_none")
	)]
	pub orientation: Option<Orientation>,
	/// Whether the element and its descendants are hidden from assistive
	/// technology (`aria-hidden`) while staying visible; decorative content is
	/// the usual case. Inherited by descendants.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_hidden", skip_serializing_if = "Option::is_none")
	)]
	pub hidden: Option<bool>,
	/// Whether a widget is being modified (`aria-busy`); assistive technology
	/// may wait for the change to complete.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_busy", skip_serializing_if = "Option::is_none")
	)]
	pub busy: Option<bool>,
	/// The check state (`aria-checked`) of a checkbox, menu item, option, or
	/// switch; widget kinds with a `checked` prop get this for free.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_checked", skip_serializing_if = "Option::is_none")
	)]
	pub checked: Option<Tristate>,
	/// The pressed state (`aria-pressed`) of a toggle button.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_pressed", skip_serializing_if = "Option::is_none")
	)]
	pub pressed: Option<Tristate>,
	/// Whether the element is selected (`aria-selected`).
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_selected", skip_serializing_if = "Option::is_none")
	)]
	pub selected: Option<bool>,
	/// Whether a control's popup is open (`aria-expanded`).
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_expanded", skip_serializing_if = "Option::is_none")
	)]
	pub expanded: Option<bool>,
	/// What the element currently represents (`aria-current`), such as the
	/// current page in navigation.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_current", skip_serializing_if = "Option::is_none")
	)]
	pub current: Option<Current>,
	/// Whether a dialog blocks the rest of the page (`aria-modal`).
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_modal", skip_serializing_if = "Option::is_none")
	)]
	pub modal: Option<bool>,
	/// Whether a field must be filled in (`aria-required`).
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_required", skip_serializing_if = "Option::is_none")
	)]
	pub required: Option<bool>,
	/// Why a field's value is invalid (`aria-invalid`).
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_invalid", skip_serializing_if = "Option::is_none")
	)]
	pub invalid: Option<Invalid>,
	/// Whether a field cannot be edited (`aria-readonly`).
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_read_only", skip_serializing_if = "Option::is_none")
	)]
	pub read_only: Option<bool>,
	/// Whether a text field accepts multiple lines (`aria-multiline`).
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_multiline", skip_serializing_if = "Option::is_none")
	)]
	pub multiline: Option<bool>,
	/// What a control opens (`aria-haspopup`).
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_has_popup", skip_serializing_if = "Option::is_none")
	)]
	pub has_popup: Option<HasPopup>,
	/// The current value of a range widget (`aria-valuenow`); sliders and
	/// progress bars get this from their state props.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_value_now", skip_serializing_if = "Option::is_none")
	)]
	pub value_now: Option<f64>,
	/// The minimum value of a range widget (`aria-valuemin`).
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_value_min", skip_serializing_if = "Option::is_none")
	)]
	pub value_min: Option<f64>,
	/// The maximum value of a range widget (`aria-valuemax`).
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_value_max", skip_serializing_if = "Option::is_none")
	)]
	pub value_max: Option<f64>,
	/// A human-readable range value (`aria-valuetext`), such as "50 percent"
	/// for a slider at half.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_value_text", skip_serializing_if = "Option::is_none")
	)]
	pub value_text: Option<String>,
	/// When changes to this region are announced (`aria-live`); the `alert`,
	/// `log`, and `status` roles imply this.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_live", skip_serializing_if = "Option::is_none")
	)]
	pub live: Option<Live>,
	/// Whether the whole region, not just the change, is announced
	/// (`aria-atomic`); the `alert` and `status` roles imply this.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_atomic", skip_serializing_if = "Option::is_none")
	)]
	pub atomic: Option<bool>,
	/// The subtree text to announce, filled by the shared resolver for live
	/// regions; backends use it to announce changes.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_live_text", skip_serializing_if = "Option::is_none")
	)]
	pub live_text: Option<String>,
	/// The ids of elements this one controls (`aria-controls`), such as a tab
	/// and its panel.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_controls", skip_serializing_if = "Vec::is_empty")
	)]
	pub controls: Vec<String>,
	/// The element's position in sequential keyboard focus (`tabindex`); a
	/// negative value removes it from tab order. Interactive roles focus
	/// automatically.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "tab_index", skip_serializing_if = "Option::is_none")
	)]
	pub tab_index: Option<i32>,
	/// Additional `aria-*` attributes, emitted verbatim on the web and ignored
	/// by native backends; `accepts_attribute` lists the rules.
	#[cfg_attr(
		feature = "serde",
		serde(rename = "aria_extra", skip_serializing_if = "BTreeMap::is_empty")
	)]
	pub extra: BTreeMap<String, String>,
}

impl Aria {
	/// Whether any property is set, so untouched elements serialize away the
	/// whole group and keep the wire fast.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self == &Self::default()
	}

	/// The role in force: the explicit role, or the widget kind's implicit
	/// role.
	#[must_use]
	pub fn effective_role(&self, kind: NativeKind) -> Option<Role> {
		self.role.or_else(|| Role::implicit(kind))
	}

	/// The live politeness in force: the explicit `live`, or the effective
	/// role's implicit politeness.
	#[must_use]
	pub fn effective_live(&self, kind: NativeKind) -> Live {
		self.live
			.or_else(|| self.effective_role(kind).and_then(Role::implicit_live))
			.unwrap_or_default()
	}

	/// Whether the effective role is a control.
	#[must_use]
	pub fn is_interactive(&self, kind: NativeKind) -> bool {
		self.effective_role(kind).is_some_and(Role::is_interactive)
	}

	/// Whether the element and its descendants hide from assistive
	/// technology. Callers pass the resolved `hidden` value, which inherits
	/// down the tree.
	#[must_use]
	pub fn is_hidden(&self) -> bool {
		self.hidden == Some(true)
	}
}

/// Decodes a role leniently: an unknown name from a newer interface becomes
/// `None` instead of failing the whole property payload.
#[cfg(feature = "serde")]
fn lenient_role<'de, D>(deserializer: D) -> Result<Option<Role>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let name = <Option<String> as serde::Deserialize>::deserialize(deserializer)?;
	Ok(name.as_deref().and_then(Role::from_wire))
}
