//! Maps shared ARIA properties onto GTK's accessible API.
//!
//! GTK's accessible API mirrors WAI-ARIA almost one to one, so the mapping
//! is direct: properties, states, and relations keep their names, explicit
//! roles map onto `AccessibleRole` (unmapped ones read as generic), and the
//! shared resolver has already folded relationship targets into concrete
//! labels and radio positions. Live-region announcements need GTK 4.14,
//! above this crate's minimum, so live regions expose their text through
//! the label and states only.

use gtk::AccessibleInvalidState;
use gtk::AccessibleRole;
use gtk::AccessibleTristate;
use gtk::Orientation;
use gtk::accessible;
use gtk::prelude::AccessibleExtManual;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::Role;
use leptoskit_native::Tristate;

/// The GTK role an ARIA role maps onto; unmapped roles read as generic.
pub(crate) fn role(role: Role) -> AccessibleRole {
	use AccessibleRole as Gtk;

	match role {
		Role::Alert => Gtk::Alert,
		Role::AlertDialog => Gtk::AlertDialog,
		Role::Banner => Gtk::Banner,
		Role::Application
		| Role::Article
		| Role::Blockquote
		| Role::Code
		| Role::Definition
		| Role::Deletion
		| Role::Emphasis
		| Role::Figure
		| Role::Generic
		| Role::Insertion
		| Role::Paragraph
		| Role::Strong
		| Role::Subscript
		| Role::Superscript
		| Role::Term => Gtk::Generic,
		Role::Button => Gtk::Button,
		Role::Caption => Gtk::Caption,
		Role::Cell => Gtk::Cell,
		Role::Checkbox => Gtk::Checkbox,
		Role::ColumnHeader => Gtk::ColumnHeader,
		Role::ComboBox => Gtk::ComboBox,
		// GTK's landmark role is abstract, so its two ARIA landmarks without a
		// concrete GTK equivalent read as generic rather than assert.
		Role::Complementary | Role::ContentInfo => Gtk::Generic,
		Role::Dialog => Gtk::Dialog,
		Role::Document => Gtk::Document,
		Role::Feed => Gtk::Feed,
		Role::Form => Gtk::Form,
		Role::Grid => Gtk::Grid,
		Role::GridCell => Gtk::GridCell,
		Role::Group => Gtk::Group,
		Role::Heading => Gtk::Heading,
		Role::Img => Gtk::Img,
		Role::Link => Gtk::Link,
		Role::List => Gtk::List,
		Role::ListBox => Gtk::ListBox,
		Role::ListItem => Gtk::ListItem,
		Role::Log => Gtk::Log,
		Role::Main => Gtk::Main,
		Role::Marquee => Gtk::Marquee,
		Role::Math => Gtk::Math,
		Role::Menu => Gtk::Menu,
		Role::MenuBar => Gtk::MenuBar,
		Role::MenuItem => Gtk::MenuItem,
		Role::MenuItemCheckbox => Gtk::MenuItemCheckbox,
		Role::MenuItemRadio => Gtk::MenuItemRadio,
		Role::Meter => Gtk::Meter,
		Role::Navigation => Gtk::Navigation,
		Role::Note => Gtk::Note,
		Role::Option => Gtk::Option,
		Role::None | Role::Presentation => Gtk::Presentation,
		Role::ProgressBar => Gtk::ProgressBar,
		Role::Radio => Gtk::Radio,
		Role::RadioGroup => Gtk::RadioGroup,
		Role::Region => Gtk::Region,
		Role::Row => Gtk::Row,
		Role::RowGroup => Gtk::RowGroup,
		Role::RowHeader => Gtk::RowHeader,
		Role::ScrollBar => Gtk::Scrollbar,
		Role::Search => Gtk::Search,
		Role::SearchBox => Gtk::SearchBox,
		Role::Separator => Gtk::Separator,
		Role::Slider => Gtk::Slider,
		Role::SpinButton => Gtk::SpinButton,
		Role::Status => Gtk::Status,
		Role::Switch => Gtk::Switch,
		Role::Tab => Gtk::Tab,
		Role::Table => Gtk::Table,
		Role::TabList => Gtk::TabList,
		Role::TabPanel => Gtk::TabPanel,
		Role::TextBox => Gtk::TextBox,
		Role::Time => Gtk::Time,
		Role::Timer => Gtk::Timer,
		Role::Toolbar => Gtk::Toolbar,
		Role::Tooltip => Gtk::Tooltip,
		Role::Tree => Gtk::Tree,
		Role::TreeGrid => Gtk::TreeGrid,
		Role::TreeItem => Gtk::TreeItem,
	}
}

fn tristate(value: Tristate) -> AccessibleTristate {
	match value {
		Tristate::True => AccessibleTristate::True,
		Tristate::False => AccessibleTristate::False,
		Tristate::Mixed => AccessibleTristate::Mixed,
	}
}

/// Applies the accessibility properties of one widget beyond its label.
///
/// The caller has already applied the accessible label; native range
/// controls keep their own values unless the app spells them out.
pub(crate) fn apply(control: &gtk::Widget, props: &NativeProps, kind: NativeKind) {
	let aria = props.aria();

	let mut properties = Vec::new();
	if let Some(description) = aria.description.as_deref() {
		properties.push(accessible::Property::Description(description));
	}
	if let Some(description) = aria.role_description.as_deref() {
		properties.push(accessible::Property::RoleDescription(description));
	}
	if let Some(level) = aria.level {
		properties.push(accessible::Property::Level(level as i32));
	}
	if let Some(orientation) = aria.orientation {
		properties.push(accessible::Property::Orientation(match orientation {
			leptoskit_native::Orientation::Horizontal => Orientation::Horizontal,
			leptoskit_native::Orientation::Vertical => Orientation::Vertical,
		}));
	}
	if let Some(read_only) = aria.read_only {
		properties.push(accessible::Property::ReadOnly(read_only));
	}
	if let Some(required) = aria.required {
		properties.push(accessible::Property::Required(required));
	}
	if let Some(modal) = aria.modal {
		properties.push(accessible::Property::Modal(modal));
	}
	if let Some(text) = aria.value_text.as_deref() {
		properties.push(accessible::Property::ValueText(text));
	}
	let native_range = matches!(
		kind,
		NativeKind::Slider | NativeKind::LinearProgressIndicator | NativeKind::ActivityIndicator
	);
	if !native_range {
		if let Some(value) = aria.value_now {
			properties.push(accessible::Property::ValueNow(value));
		}
		if let Some(value) = aria.value_min {
			properties.push(accessible::Property::ValueMin(value));
		}
		if let Some(value) = aria.value_max {
			properties.push(accessible::Property::ValueMax(value));
		}
	}
	if !properties.is_empty() {
		control.update_property(&properties);
	}

	let mut states = Vec::new();
	if let Some(checked) = aria.checked {
		states.push(accessible::State::Checked(tristate(checked)));
	}
	if let Some(pressed) = aria.pressed {
		states.push(accessible::State::Pressed(tristate(pressed)));
	}
	if let Some(selected) = aria.selected {
		states.push(accessible::State::Selected(Some(selected)));
	}
	if let Some(expanded) = aria.expanded {
		states.push(accessible::State::Expanded(Some(expanded)));
	}
	if let Some(busy) = aria.busy {
		states.push(accessible::State::Busy(busy));
	}
	if let Some(invalid) = aria.invalid {
		states.push(accessible::State::Invalid(match invalid {
			leptoskit_native::Invalid::False => AccessibleInvalidState::False,
			leptoskit_native::Invalid::True => AccessibleInvalidState::True,
			leptoskit_native::Invalid::Grammar => AccessibleInvalidState::Grammar,
			leptoskit_native::Invalid::Spelling => AccessibleInvalidState::Spelling,
		}));
	}
	if let Some(hidden) = aria.hidden {
		states.push(accessible::State::Hidden(hidden));
	}
	if !states.is_empty() {
		control.update_state(&states);
	}

	let mut relations = Vec::new();
	if let Some(position) = aria.pos_in_set {
		relations.push(accessible::Relation::PosInSet(position as i32));
	}
	if let Some(size) = aria.set_size {
		relations.push(accessible::Relation::SetSize(size as i32));
	}
	if !relations.is_empty() {
		control.update_relation(&relations);
	}
}
