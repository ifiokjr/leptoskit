//! Validated values shared by native and browser controls.

use std::fmt;

use crate::NativeKind;

/// A structurally invalid shared control tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlTreeError {
	/// A leaf widget cannot contain child widgets.
	Children { kind: NativeKind },
	/// Radio options require an explicit named group.
	MissingRadioGroup,
	/// A controlled radio group cannot select more than one option.
	MultipleSelectedRadio { group: String },
	/// A button's decorative content cannot contain another interactive widget.
	InteractiveDescendant {
		container: NativeKind,
		descendant: NativeKind,
	},
}

impl fmt::Display for ControlTreeError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Children { kind } => {
				write!(
					formatter,
					"{kind} is a leaf control and cannot contain children"
				)
			}
			Self::MissingRadioGroup => formatter.write_str("radio requires an explicit RadioGroup"),
			Self::MultipleSelectedRadio { group } => {
				write!(
					formatter,
					"radio group `{group}` has more than one selected option"
				)
			}
			Self::InteractiveDescendant {
				container,
				descendant,
			} => {
				write!(
					formatter,
					"{container} cannot contain interactive {descendant} descendants"
				)
			}
		}
	}
}

impl std::error::Error for ControlTreeError {}

/// Invalid public control configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlValueError {
	/// A slider requires finite bounds and a representable optional step grid.
	Slider,
	/// Determinate progress must be finite and between zero and one.
	Progress,
	/// A radio group needs a nonempty name of at most 128 bytes.
	RadioGroup,
}

impl fmt::Display for ControlValueError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(match self {
			Self::Slider => {
				"slider requires finite increasing bounds, a positive step no larger than the \
				 range, and an in-range value aligned to that step"
			}
			Self::Progress => "progress must be finite and between zero and one",
			Self::RadioGroup => "radio group name must contain between 1 and 128 bytes",
		})
	}
}

impl std::error::Error for ControlValueError {}

/// An explicit radio group, scoped to one rendered tree.
///
/// Applications retain the selected option and rebuild all group members after
/// a `Toggle { checked: true }` proposal. At most one member may be selected.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(try_from = "String", into = "String")
)]
pub struct RadioGroup(String);

impl RadioGroup {
	/// Creates a stable group name shared by related radio controls.
	pub fn new(name: impl Into<String>) -> Result<Self, ControlValueError> {
		let name = name.into();

		if name.is_empty() || name.len() > 128 {
			return Err(ControlValueError::RadioGroup);
		}

		Ok(Self(name))
	}

	/// The group name used by the platform renderer.
	pub fn name(&self) -> &str {
		&self.0
	}
}

impl TryFrom<String> for RadioGroup {
	type Error = ControlValueError;

	fn try_from(value: String) -> Result<Self, Self::Error> {
		Self::new(value)
	}
}

impl From<RadioGroup> for String {
	fn from(value: RadioGroup) -> Self {
		value.0
	}
}

/// A finite, controlled slider value and its inclusive range and step grid.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(try_from = "SliderFields", into = "SliderFields")
)]
pub struct SliderState {
	min: f64,
	max: f64,
	value: f64,
	step: Option<f64>,
}

impl SliderState {
	/// Validates a slider. `None` permits continuous values. With a step,
	/// values lie on `min + n * step`; the upper bound need not itself be
	/// on that grid. The grid must be representable in `f64`, with at most
	/// 2⁵³ intervals.
	pub fn new(
		min: f64,
		max: f64,
		value: f64,
		step: Option<f64>,
	) -> Result<Self, ControlValueError> {
		let span = max - min;

		if ![min, max, value, span].into_iter().all(f64::is_finite)
			|| span <= 0.0
			|| value < min
			|| value > max
		{
			return Err(ControlValueError::Slider);
		}

		if let Some(step) = step {
			let index = (value - min) / step;

			if !step.is_finite()
				|| step <= 0.0
				|| step > span
				|| !(span / step).is_finite()
				|| span / step > (1_u64 << 53) as f64
				|| min + step <= min
				|| !index.is_finite()
				|| (index - index.round()).abs() > 1e-7
			{
				return Err(ControlValueError::Slider);
			}
		}

		Ok(Self {
			min,
			max,
			value,
			step,
		})
	}

	/// The inclusive lower bound.
	pub const fn min(self) -> f64 {
		self.min
	}

	/// The inclusive upper bound.
	pub const fn max(self) -> f64 {
		self.max
	}

	/// The committed value.
	pub const fn value(self) -> f64 {
		self.value
	}

	/// The positive distance between selectable values, or `None` for a
	/// continuous slider.
	pub const fn step(self) -> Option<f64> {
		self.step
	}

	/// Snaps a finite user proposal to the nearest permitted step.
	pub fn proposed_value(self, value: f64) -> Option<f64> {
		if !value.is_finite() {
			return None;
		}

		let Some(step) = self.step else {
			return Some(value.clamp(self.min, self.max));
		};
		let steps = (self.max - self.min) / step;
		// Decimal steps such as 0.1 may put an exact mathematical endpoint
		// just below its integer index in binary floating point.
		let last = if (steps - steps.round()).abs() <= 1e-7 {
			steps.round()
		} else {
			steps.floor()
		};
		let index = ((value.clamp(self.min, self.max) - self.min) / step)
			.round()
			.clamp(0.0, last);
		Some((self.min + index * step).clamp(self.min, self.max))
	}
}

impl Default for SliderState {
	fn default() -> Self {
		Self {
			min: 0.0,
			max: 1.0,
			value: 0.0,
			step: None,
		}
	}
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
struct SliderFields {
	min: f64,
	max: f64,
	value: f64,
	step: Option<f64>,
}

#[cfg(feature = "serde")]
impl TryFrom<SliderFields> for SliderState {
	type Error = ControlValueError;

	fn try_from(value: SliderFields) -> Result<Self, Self::Error> {
		Self::new(value.min, value.max, value.value, value.step)
	}
}

#[cfg(feature = "serde")]
impl From<SliderState> for SliderFields {
	fn from(value: SliderState) -> Self {
		Self {
			min: value.min,
			max: value.max,
			value: value.value,
			step: value.step,
		}
	}
}

/// A finite completion fraction in the inclusive range zero to one.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(try_from = "f64", into = "f64")
)]
pub struct ProgressValue(f64);

impl ProgressValue {
	/// Validates a completion fraction.
	pub fn new(value: f64) -> Result<Self, ControlValueError> {
		if !value.is_finite() || !(0.0..=1.0).contains(&value) {
			return Err(ControlValueError::Progress);
		}

		Ok(Self(value))
	}

	/// The completion fraction.
	pub const fn get(self) -> f64 {
		self.0
	}
}

impl TryFrom<f64> for ProgressValue {
	type Error = ControlValueError;

	fn try_from(value: f64) -> Result<Self, Self::Error> {
		Self::new(value)
	}
}

impl From<ProgressValue> for f64 {
	fn from(value: ProgressValue) -> Self {
		value.0
	}
}

/// A progress indicator's known fraction or indeterminate loading state.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "snake_case")
)]
pub enum Progress {
	/// Loading without a known completion fraction.
	#[default]
	Indeterminate,
	/// Loading with a known completion fraction.
	Determinate(ProgressValue),
}
