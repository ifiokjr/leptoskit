//! A document's variables: inputs the user changes, formulas over them, and
//! data the application supplies.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use leptoskit_native::reactive::Get;
use leptoskit_native::reactive::Set;
use reactive_graph::signal::ArcRwSignal;
use reactive_graph::traits::GetUntracked;

use crate::expr::Budget;
use crate::expr::Expr;
use crate::expr::FormulaError;
use crate::expr::Scope;
use crate::value::Value;

/// How deeply formulas may refer to one another.
const MAX_FORMULA_CHAIN: u32 = 16;

/// One variable.
#[derive(Clone)]
pub(crate) enum Variable {
	/// Declared by the document with `<Var>`; inputs and buttons write it.
	Input {
		signal: ArcRwSignal<Value>,
		/// The value the document declared, so a document that declares a
		/// different one resets it and one that repeats itself does not.
		initial: Value,
	},
	/// Declared by the document with `<Let>`: a formula over other variables.
	Formula(Arc<Expr>),
	/// Supplied by the application; the document reads it and cannot write it.
	Data(ArcRwSignal<Value>),
}

/// Every variable, shared by every render of the documents one
/// [`AgentUi`](crate::AgentUi) shows.
///
/// Values live in signals, so a formula or a chart that reads one follows it,
/// and they outlive a render, so a user's input survives the agent sending
/// the document again with more of it written.
#[derive(Clone, Default)]
pub(crate) struct Store {
	variables: Arc<Mutex<HashMap<String, Variable>>>,
}

/// Why a declaration was refused.
pub(crate) enum Declare {
	Ok,
	Refused(String),
}

impl Store {
	fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Variable>> {
		self.variables
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
	}

	pub(crate) fn get(&self, name: &str) -> Option<Variable> {
		self.lock().get(name).cloned()
	}

	/// Declares an input variable, keeping the user's value unless the
	/// document now declares a different starting value.
	pub(crate) fn declare_input(&self, name: &str, initial: Value, max: usize) -> Declare {
		let mut variables = self.lock();
		let full = variables.len() >= max;

		match variables.get_mut(name) {
			Some(Variable::Input {
				signal,
				initial: declared,
			}) => {
				if *declared != initial {
					*declared = initial.clone();
					signal.set(initial);
				}
				Declare::Ok
			}
			Some(Variable::Data(_)) => {
				Declare::Refused(format!(
					"`{name}` is the application's data and cannot be redeclared"
				))
			}
			Some(slot @ Variable::Formula(_)) => {
				*slot = Variable::Input {
					signal: ArcRwSignal::new(initial.clone()),
					initial,
				};
				Declare::Ok
			}
			None if full => Declare::Refused(format!("a document is limited to {max} variables")),
			None => {
				variables.insert(
					name.to_owned(),
					Variable::Input {
						signal: ArcRwSignal::new(initial.clone()),
						initial,
					},
				);
				Declare::Ok
			}
		}
	}

	/// Declares or replaces a formula variable.
	pub(crate) fn declare_formula(&self, name: &str, expr: Expr, max: usize) -> Declare {
		let mut variables = self.lock();

		match variables.get(name) {
			Some(Variable::Data(_)) => {
				Declare::Refused(format!(
					"`{name}` is the application's data and cannot be redeclared"
				))
			}
			None if variables.len() >= max => {
				Declare::Refused(format!("a document is limited to {max} variables"))
			}
			_ => {
				variables.insert(name.to_owned(), Variable::Formula(Arc::new(expr)));
				Declare::Ok
			}
		}
	}

	/// Sets application data, creating it the first time.
	pub(crate) fn set_data(&self, name: &str, value: Value) {
		let mut variables = self.lock();

		match variables.get(name) {
			Some(Variable::Data(signal)) => signal.set(value),
			_ => {
				variables.insert(name.to_owned(), Variable::Data(ArcRwSignal::new(value)));
			}
		}
	}

	/// Writes an input variable, the only kind a document can change.
	pub(crate) fn write(&self, name: &str, value: Value) -> Result<(), String> {
		match self.get(name) {
			Some(Variable::Input { signal, .. }) => {
				signal.set(value);
				Ok(())
			}
			Some(Variable::Formula(_)) => Err(format!("`{name}` is a formula and cannot be set")),
			Some(Variable::Data(_)) => {
				Err(format!(
					"`{name}` is the application's data and cannot be set"
				))
			}
			None => {
				Err(format!(
					"there is no variable `{name}`; declare it with <Var>"
				))
			}
		}
	}

	/// The current value of every variable, formulas evaluated.
	pub(crate) fn snapshot(&self, fuel: usize, max_list: usize) -> BTreeMap<String, Value> {
		let names: Vec<String> = self.lock().keys().cloned().collect();

		names
			.into_iter()
			.filter_map(|name| {
				let mut budget = Budget::new(fuel, max_list);
				let value = self.variable(&name, &mut budget).ok().flatten()?;
				Some((name, value))
			})
			.collect()
	}

	/// The input variables' values, which is the state a document's user can
	/// change.
	pub(crate) fn inputs(&self) -> BTreeMap<String, Value> {
		self.lock()
			.iter()
			.filter_map(|(name, variable)| {
				match variable {
					Variable::Input { signal, .. } => Some((name.clone(), signal.get_untracked())),
					_ => None,
				}
			})
			.collect()
	}

	/// The application's data, by name, for the language reference.
	pub(crate) fn data_summary(&self) -> BTreeMap<String, Value> {
		self.lock()
			.iter()
			.filter_map(|(name, variable)| {
				match variable {
					Variable::Data(signal) => Some((name.clone(), signal.get_untracked())),
					_ => None,
				}
			})
			.collect()
	}

	pub(crate) fn clear_document(&self) {
		self.lock()
			.retain(|_, variable| matches!(variable, Variable::Data(_)));
	}
}

impl Scope for Store {
	fn variable(&self, name: &str, budget: &mut Budget) -> Result<Option<Value>, FormulaError> {
		// Clone the variable out before reading it: a formula reads other
		// variables, which takes the lock again.
		match self.get(name) {
			None => Ok(None),
			Some(Variable::Input { signal, .. } | Variable::Data(signal)) => Ok(Some(signal.get())),
			Some(Variable::Formula(expr)) => {
				if budget.depth >= MAX_FORMULA_CHAIN {
					return Err(FormulaError(format!(
						"`{name}` refers to itself, or formulas refer to each other too deeply"
					)));
				}

				budget.depth += 1;
				let value = expr.eval(self, budget);
				budget.depth -= 1;
				value.map(Some)
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn eval(store: &Store, source: &str) -> Result<Value, FormulaError> {
		Expr::parse(source)?.eval(store, &mut Budget::new(10_000, 1000))
	}

	#[test]
	fn formulas_follow_inputs() {
		let store = Store::default();
		assert!(matches!(
			store.declare_input("rent", Value::Number(1800.0), 10),
			Declare::Ok
		));
		store.declare_formula("yearly", Expr::parse("rent * 12").unwrap(), 10);
		assert_eq!(eval(&store, "yearly").unwrap(), Value::Number(21600.0));

		store.write("rent", Value::Number(2000.0)).unwrap();
		assert_eq!(eval(&store, "yearly").unwrap(), Value::Number(24000.0));
	}

	#[test]
	fn redeclaring_keeps_the_users_value_until_the_document_changes_it() {
		let store = Store::default();
		store.declare_input("rent", Value::Number(1800.0), 10);
		store.write("rent", Value::Number(2000.0)).unwrap();

		store.declare_input("rent", Value::Number(1800.0), 10);
		assert_eq!(eval(&store, "rent").unwrap(), Value::Number(2000.0));

		store.declare_input("rent", Value::Number(1500.0), 10);
		assert_eq!(eval(&store, "rent").unwrap(), Value::Number(1500.0));
	}

	#[test]
	fn the_document_cannot_write_application_data() {
		let store = Store::default();
		store.set_data("income", Value::Number(5000.0));
		assert!(store.write("income", Value::Number(1.0)).is_err());
		assert!(matches!(
			store.declare_input("income", Value::Number(1.0), 10),
			Declare::Refused(_)
		));
		assert_eq!(eval(&store, "income").unwrap(), Value::Number(5000.0));
	}

	#[test]
	fn formulas_that_refer_to_themselves_fail() {
		let store = Store::default();
		store.declare_formula("a", Expr::parse("b + 1").unwrap(), 10);
		store.declare_formula("b", Expr::parse("a + 1").unwrap(), 10);
		assert!(eval(&store, "a").is_err());
	}

	#[test]
	fn limits_how_many_variables_a_document_declares() {
		let store = Store::default();
		store.declare_input("a", Value::Null, 1);
		assert!(matches!(
			store.declare_input("b", Value::Null, 1),
			Declare::Refused(_)
		));
	}
}
