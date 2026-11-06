//! Formulas: the small, pure expression language inside `{…}`.
//!
//! A formula reads variables and calls built-in functions. It cannot loop,
//! define functions, or reach anything outside the document, and every
//! evaluation runs on a fixed amount of fuel, so a formula the agent writes
//! always finishes quickly.

use std::fmt;

use crate::value::Value;
use crate::value::format_number;
use crate::value::group_thousands;

/// The longest formula, in bytes.
const MAX_SOURCE: usize = 2048;
/// How deeply a formula may nest.
const MAX_DEPTH: u32 = 32;

/// A parsed formula.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
	Literal(Value),
	Variable(String),
	List(Vec<Self>),
	Unary(UnaryOp, Box<Self>),
	Binary(BinaryOp, Box<Self>, Box<Self>),
	And(Box<Self>, Box<Self>),
	Or(Box<Self>, Box<Self>),
	Conditional(Box<Self>, Box<Self>, Box<Self>),
	Field(Box<Self>, String),
	Index(Box<Self>, Box<Self>),
	Call(String, Vec<Self>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnaryOp {
	Negate,
	Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryOp {
	Add,
	Subtract,
	Multiply,
	Divide,
	Remainder,
	Equal,
	NotEqual,
	Less,
	LessEqual,
	Greater,
	GreaterEqual,
}

/// Why a formula failed to parse or evaluate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaError(pub(crate) String);

impl fmt::Display for FormulaError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.0)
	}
}

impl std::error::Error for FormulaError {}

fn error<T>(message: impl Into<String>) -> Result<T, FormulaError> {
	Err(FormulaError(message.into()))
}

impl Expr {
	/// Parses `source`.
	pub(crate) fn parse(source: &str) -> Result<Self, FormulaError> {
		if source.len() > MAX_SOURCE {
			return error(format!("a formula is limited to {MAX_SOURCE} bytes"));
		}

		let tokens = tokenize(source)?;
		let mut parser = Parser {
			tokens,
			position: 0,
			depth: 0,
		};
		let expr = parser.expression()?;

		match parser.peek() {
			None => Ok(expr),
			Some(token) => error(format!("unexpected `{token}`")),
		}
	}

	/// Whether the formula reads no variables, so its value never changes.
	pub(crate) fn is_constant(&self) -> bool {
		match self {
			Self::Literal(_) => true,
			Self::Variable(_) => false,
			Self::List(items) | Self::Call(_, items) => items.iter().all(Self::is_constant),
			Self::Unary(_, inner) | Self::Field(inner, _) => inner.is_constant(),
			Self::Binary(_, left, right)
			| Self::And(left, right)
			| Self::Or(left, right)
			| Self::Index(left, right) => left.is_constant() && right.is_constant(),
			Self::Conditional(condition, then, otherwise) => {
				condition.is_constant() && then.is_constant() && otherwise.is_constant()
			}
		}
	}

	/// The variable this formula names, when it is nothing but a name.
	pub(crate) fn as_variable(&self) -> Option<&str> {
		match self {
			Self::Variable(name) => Some(name),
			_ => None,
		}
	}
}

// --- Tokens ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
	Number(f64),
	Text(String),
	Name(String),
	Symbol(&'static str),
}

impl fmt::Display for Token {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Number(number) => write!(formatter, "{number}"),
			Self::Text(text) => write!(formatter, "{text:?}"),
			Self::Name(name) => formatter.write_str(name),
			Self::Symbol(symbol) => formatter.write_str(symbol),
		}
	}
}

/// Symbols, longest first so `<=` wins over `<`.
const SYMBOLS: [&str; 22] = [
	"==", "!=", "<=", ">=", "&&", "||", "+", "-", "*", "/", "%", "<", ">", "!", "?", ":", "(", ")",
	"[", "]", ",", ".",
];

fn tokenize(source: &str) -> Result<Vec<Token>, FormulaError> {
	let mut tokens = Vec::new();
	let mut rest = source;

	while let Some(character) = rest.chars().next() {
		if character.is_whitespace() {
			rest = &rest[character.len_utf8()..];
		} else if character.is_ascii_digit()
			|| (character == '.' && rest[1..].starts_with(|next: char| next.is_ascii_digit()))
		{
			let end = rest
				.find(|next: char| !(next.is_ascii_digit() || next == '.' || next == '_'))
				.unwrap_or(rest.len());
			let digits = rest[..end].replace('_', "");
			let number = digits
				.parse()
				.map_err(|_| FormulaError(format!("`{}` is not a number", &rest[..end])))?;
			tokens.push(Token::Number(number));
			rest = &rest[end..];
		} else if character == '"' || character == '\'' {
			let (text, remaining) = string(rest, character)?;
			tokens.push(Token::Text(text));
			rest = remaining;
		} else if character.is_alphabetic() || character == '_' {
			let end = rest
				.find(|next: char| !(next.is_alphanumeric() || next == '_'))
				.unwrap_or(rest.len());
			tokens.push(Token::Name(rest[..end].to_owned()));
			rest = &rest[end..];
		} else if let Some(symbol) = SYMBOLS.iter().find(|symbol| rest.starts_with(**symbol)) {
			tokens.push(Token::Symbol(symbol));
			rest = &rest[symbol.len()..];
		} else {
			return error(format!("unexpected character `{character}`"));
		}
	}

	Ok(tokens)
}

/// Reads a quoted string at the start of `source`, returning its text and
/// what follows it.
fn string(source: &str, quote: char) -> Result<(String, &str), FormulaError> {
	let mut text = String::new();
	let mut characters = source.char_indices().skip(1);

	while let Some((index, character)) = characters.next() {
		if character == quote {
			return Ok((text, &source[index + character.len_utf8()..]));
		}

		if character == '\\' {
			match characters.next() {
				Some((_, 'n')) => text.push('\n'),
				Some((_, escaped)) => text.push(escaped),
				None => break,
			}
		} else {
			text.push(character);
		}
	}

	error("text is missing its closing quote")
}

// --- Parser ---------------------------------------------------------------

struct Parser {
	tokens: Vec<Token>,
	position: usize,
	depth: u32,
}

impl Parser {
	fn peek(&self) -> Option<&Token> {
		self.tokens.get(self.position)
	}

	fn next(&mut self) -> Option<Token> {
		let token = self.tokens.get(self.position).cloned();
		self.position += 1;
		token
	}

	fn eat(&mut self, symbol: &str) -> bool {
		if matches!(self.peek(), Some(Token::Symbol(found)) if *found == symbol) {
			self.position += 1;
			true
		} else {
			false
		}
	}

	fn expect(&mut self, symbol: &str) -> Result<(), FormulaError> {
		if self.eat(symbol) {
			Ok(())
		} else {
			match self.peek() {
				Some(token) => error(format!("expected `{symbol}`, found `{token}`")),
				None => error(format!("expected `{symbol}` at the end")),
			}
		}
	}

	fn nested<T>(
		&mut self,
		parse: impl FnOnce(&mut Self) -> Result<T, FormulaError>,
	) -> Result<T, FormulaError> {
		self.depth += 1;
		if self.depth > MAX_DEPTH {
			return error("the formula nests too deeply");
		}

		let result = parse(self);
		self.depth -= 1;
		result
	}

	fn expression(&mut self) -> Result<Expr, FormulaError> {
		self.nested(|parser| {
			let condition = parser.or()?;
			if !parser.eat("?") {
				return Ok(condition);
			}

			let then = parser.expression()?;
			parser.expect(":")?;
			let otherwise = parser.expression()?;
			Ok(Expr::Conditional(
				Box::new(condition),
				Box::new(then),
				Box::new(otherwise),
			))
		})
	}

	fn or(&mut self) -> Result<Expr, FormulaError> {
		let mut left = self.and()?;
		while self.eat("||") {
			left = Expr::Or(Box::new(left), Box::new(self.and()?));
		}
		Ok(left)
	}

	fn and(&mut self) -> Result<Expr, FormulaError> {
		let mut left = self.binary(0)?;
		while self.eat("&&") {
			left = Expr::And(Box::new(left), Box::new(self.binary(0)?));
		}
		Ok(left)
	}

	/// Binary operators by precedence level: equality, comparison, sums, and
	/// products.
	fn binary(&mut self, level: usize) -> Result<Expr, FormulaError> {
		const LEVELS: [&[(&str, BinaryOp)]; 4] = [
			&[("==", BinaryOp::Equal), ("!=", BinaryOp::NotEqual)],
			&[
				("<=", BinaryOp::LessEqual),
				(">=", BinaryOp::GreaterEqual),
				("<", BinaryOp::Less),
				(">", BinaryOp::Greater),
			],
			&[("+", BinaryOp::Add), ("-", BinaryOp::Subtract)],
			&[
				("*", BinaryOp::Multiply),
				("/", BinaryOp::Divide),
				("%", BinaryOp::Remainder),
			],
		];

		let Some(operators) = LEVELS.get(level) else {
			return self.unary();
		};

		let mut left = self.binary(level + 1)?;
		'operators: loop {
			for (symbol, op) in *operators {
				if self.eat(symbol) {
					let right = self.binary(level + 1)?;
					left = Expr::Binary(*op, Box::new(left), Box::new(right));
					continue 'operators;
				}
			}

			return Ok(left);
		}
	}

	fn unary(&mut self) -> Result<Expr, FormulaError> {
		if self.eat("-") {
			return self
				.nested(|parser| Ok(Expr::Unary(UnaryOp::Negate, Box::new(parser.unary()?))));
		}

		if self.eat("!") {
			return self.nested(|parser| Ok(Expr::Unary(UnaryOp::Not, Box::new(parser.unary()?))));
		}

		self.postfix()
	}

	fn postfix(&mut self) -> Result<Expr, FormulaError> {
		let mut expr = self.primary()?;

		loop {
			if self.eat(".") {
				match self.next() {
					Some(Token::Name(field)) => expr = Expr::Field(Box::new(expr), field),
					_ => return error("expected a field name after `.`"),
				}
			} else if self.eat("[") {
				let index = self.expression()?;
				self.expect("]")?;
				expr = Expr::Index(Box::new(expr), Box::new(index));
			} else {
				return Ok(expr);
			}
		}
	}

	fn primary(&mut self) -> Result<Expr, FormulaError> {
		match self.next() {
			Some(Token::Number(number)) => Ok(Expr::Literal(Value::Number(number))),
			Some(Token::Text(text)) => Ok(Expr::Literal(Value::Text(text))),
			Some(Token::Name(name)) => self.name(name),
			Some(Token::Symbol("(")) => {
				let expr = self.expression()?;
				self.expect(")")?;
				Ok(expr)
			}
			Some(Token::Symbol("[")) => Ok(Expr::List(self.items("]")?)),
			Some(token) => error(format!("unexpected `{token}`")),
			None => error("the formula ends too early"),
		}
	}

	fn name(&mut self, name: String) -> Result<Expr, FormulaError> {
		match name.as_str() {
			"true" => return Ok(Expr::Literal(Value::Bool(true))),
			"false" => return Ok(Expr::Literal(Value::Bool(false))),
			"null" => return Ok(Expr::Literal(Value::Null)),
			_ => {}
		}

		if !self.eat("(") {
			return Ok(Expr::Variable(name));
		}

		if !FUNCTIONS.iter().any(|function| function.name == name) {
			return error(format!("there is no function `{name}`"));
		}

		Ok(Expr::Call(name, self.items(")")?))
	}

	/// Comma-separated expressions up to `close`, which may follow a trailing
	/// comma.
	fn items(&mut self, close: &str) -> Result<Vec<Expr>, FormulaError> {
		let mut items = Vec::new();

		while !self.eat(close) {
			items.push(self.expression()?);
			if !self.eat(",") {
				self.expect(close)?;
				break;
			}
		}

		Ok(items)
	}
}

// --- Evaluation -----------------------------------------------------------

/// Where a formula's variables come from.
pub(crate) trait Scope {
	/// The value of `name`, or `None` when nothing by that name exists.
	fn variable(&self, name: &str, budget: &mut Budget) -> Result<Option<Value>, FormulaError>;
}

/// The work one evaluation may do, so every formula finishes quickly.
#[derive(Debug)]
pub(crate) struct Budget {
	fuel: usize,
	max_list: usize,
	/// How many derived variables are being evaluated inside one another.
	pub(crate) depth: u32,
}

impl Budget {
	pub(crate) const fn new(fuel: usize, max_list: usize) -> Self {
		Self {
			fuel,
			max_list,
			depth: 0,
		}
	}

	fn spend(&mut self, amount: usize) -> Result<(), FormulaError> {
		match self.fuel.checked_sub(amount.max(1)) {
			Some(left) => {
				self.fuel = left;
				Ok(())
			}
			None => error("the formula does too much work"),
		}
	}

	fn list(&mut self, items: Vec<Value>) -> Result<Value, FormulaError> {
		if items.len() > self.max_list {
			return error(format!("a list is limited to {} items", self.max_list));
		}

		self.spend(items.len())?;
		Ok(Value::List(items))
	}
}

impl Expr {
	/// The formula's value in `scope`.
	pub(crate) fn eval(
		&self,
		scope: &dyn Scope,
		budget: &mut Budget,
	) -> Result<Value, FormulaError> {
		budget.spend(1)?;

		match self {
			Self::Literal(value) => Ok(value.clone()),
			Self::Variable(name) => {
				match scope.variable(name, budget)? {
					Some(value) => Ok(value),
					None => error(format!("there is no variable `{name}`")),
				}
			}
			Self::List(items) => {
				let values = items
					.iter()
					.map(|item| item.eval(scope, budget))
					.collect::<Result<Vec<_>, _>>()?;
				budget.list(values)
			}
			Self::Unary(op, inner) => unary(*op, &inner.eval(scope, budget)?),
			Self::Binary(op, left, right) => {
				let left = left.eval(scope, budget)?;
				let right = right.eval(scope, budget)?;
				binary(*op, &left, &right, budget)
			}
			Self::And(left, right) => {
				let left = left.eval(scope, budget)?;
				if left.truthy() {
					right.eval(scope, budget)
				} else {
					Ok(left)
				}
			}
			Self::Or(left, right) => {
				let left = left.eval(scope, budget)?;
				if left.truthy() {
					Ok(left)
				} else {
					right.eval(scope, budget)
				}
			}
			Self::Conditional(condition, then, otherwise) => {
				if condition.eval(scope, budget)?.truthy() {
					then.eval(scope, budget)
				} else {
					otherwise.eval(scope, budget)
				}
			}
			Self::Field(inner, field) => {
				match inner.eval(scope, budget)? {
					Value::Record(mut fields) => Ok(fields.remove(field).unwrap_or_default()),
					Value::List(items) if field == "length" => {
						Ok(Value::Number(items.len() as f64))
					}
					other => error(format!("a {} has no field `{field}`", other.kind())),
				}
			}
			Self::Index(inner, index) => {
				let inner = inner.eval(scope, budget)?;
				let index = index.eval(scope, budget)?;
				get(&inner, &index)
			}
			Self::Call(name, arguments) => {
				let arguments = arguments
					.iter()
					.map(|argument| argument.eval(scope, budget))
					.collect::<Result<Vec<_>, _>>()?;
				call(name, arguments, budget)
			}
		}
	}
}

fn number(value: &Value, what: &str) -> Result<f64, FormulaError> {
	value
		.as_number()
		.ok_or_else(|| FormulaError(format!("{what} needs a number, not {}", describe(value))))
}

fn describe(value: &Value) -> String {
	match value {
		Value::Text(text) => format!("the text {text:?}"),
		Value::Null => "nothing".to_owned(),
		other => format!("a {}", other.kind()),
	}
}

fn unary(op: UnaryOp, value: &Value) -> Result<Value, FormulaError> {
	match op {
		UnaryOp::Not => Ok(Value::Bool(!value.truthy())),
		UnaryOp::Negate => {
			match value {
				Value::List(items) => {
					items
						.iter()
						.map(|item| unary(op, item))
						.collect::<Result<Vec<_>, _>>()
						.map(Value::List)
				}
				other => Ok(Value::Number(-number(other, "`-`")?)),
			}
		}
	}
}

fn binary(
	op: BinaryOp,
	left: &Value,
	right: &Value,
	budget: &mut Budget,
) -> Result<Value, FormulaError> {
	match op {
		BinaryOp::Equal => return Ok(Value::Bool(equal(left, right))),
		BinaryOp::NotEqual => return Ok(Value::Bool(!equal(left, right))),
		_ => {}
	}

	// Lists combine item by item, and with a single value as every item:
	// `[1, 2] * 12` is `[12, 24]`.
	match (left, right) {
		(Value::List(lefts), Value::List(rights)) => {
			if lefts.len() != rights.len() {
				return error(format!(
					"cannot combine lists of {} and {} items",
					lefts.len(),
					rights.len()
				));
			}

			let items = lefts
				.iter()
				.zip(rights)
				.map(|(left, right)| binary(op, left, right, budget))
				.collect::<Result<Vec<_>, _>>()?;
			return budget.list(items);
		}
		(Value::List(lefts), single) => {
			let items = lefts
				.iter()
				.map(|left| binary(op, left, single, budget))
				.collect::<Result<Vec<_>, _>>()?;
			return budget.list(items);
		}
		(single, Value::List(rights)) => {
			let items = rights
				.iter()
				.map(|right| binary(op, single, right, budget))
				.collect::<Result<Vec<_>, _>>()?;
			return budget.list(items);
		}
		_ => {}
	}

	if op == BinaryOp::Add
		&& (matches!(left, Value::Text(_)) || matches!(right, Value::Text(_)))
		&& (left.as_number().is_none() || right.as_number().is_none())
	{
		let joined = format!("{left}{right}");
		budget.spend(joined.len() / 64)?;
		return Ok(Value::Text(joined));
	}

	let symbol = match op {
		BinaryOp::Add => "`+`",
		BinaryOp::Subtract => "`-`",
		BinaryOp::Multiply => "`*`",
		BinaryOp::Divide => "`/`",
		BinaryOp::Remainder => "`%`",
		_ => "a comparison",
	};
	let a = number(left, symbol)?;
	let b = number(right, symbol)?;

	Ok(match op {
		BinaryOp::Add => Value::Number(a + b),
		BinaryOp::Subtract => Value::Number(a - b),
		BinaryOp::Multiply => Value::Number(a * b),
		// Dividing by zero gives nothing rather than infinity, so a total that
		// is still zero shows as empty rather than as a broken number.
		BinaryOp::Divide | BinaryOp::Remainder if b == 0.0 => Value::Null,
		BinaryOp::Divide => Value::Number(a / b),
		BinaryOp::Remainder => Value::Number(a % b),
		BinaryOp::Less => Value::Bool(a < b),
		BinaryOp::LessEqual => Value::Bool(a <= b),
		BinaryOp::Greater => Value::Bool(a > b),
		BinaryOp::GreaterEqual => Value::Bool(a >= b),
		BinaryOp::Equal | BinaryOp::NotEqual => unreachable!("handled above"),
	})
}

/// Equality that treats `"5"` and `5` as different but `1800` and `1800.0` as
/// the same.
fn equal(left: &Value, right: &Value) -> bool {
	match (left, right) {
		(Value::Number(a), Value::Number(b)) => (a - b).abs() < f64::EPSILON,
		_ => left == right,
	}
}

fn get(container: &Value, index: &Value) -> Result<Value, FormulaError> {
	match (container, index) {
		(Value::List(items), index) => {
			let position = number(index, "a list index")?;
			if position < 0.0 || position.fract() != 0.0 {
				return Ok(Value::Null);
			}

			Ok(items.get(position as usize).cloned().unwrap_or_default())
		}
		(Value::Record(fields), Value::Text(key)) => {
			Ok(fields.get(key).cloned().unwrap_or_default())
		}
		(other, _) => error(format!("cannot index into {}", describe(other))),
	}
}

// --- Functions ------------------------------------------------------------

/// A built-in function, with what the language reference says about it.
pub(crate) struct Function {
	pub(crate) name: &'static str,
	pub(crate) signature: &'static str,
	pub(crate) summary: &'static str,
}

/// Every function a formula can call.
pub(crate) const FUNCTIONS: &[Function] = &[
	Function {
		name: "sum",
		signature: "sum(list)",
		summary: "The total of a list of numbers.",
	},
	Function {
		name: "avg",
		signature: "avg(list)",
		summary: "The mean of a list of numbers.",
	},
	Function {
		name: "min",
		signature: "min(list) or min(a, b, …)",
		summary: "The smallest number.",
	},
	Function {
		name: "max",
		signature: "max(list) or max(a, b, …)",
		summary: "The largest number.",
	},
	Function {
		name: "len",
		signature: "len(list or text)",
		summary: "How many items, or characters.",
	},
	Function {
		name: "round",
		signature: "round(x, digits = 0)",
		summary: "Rounds to a number of decimal places.",
	},
	Function {
		name: "floor",
		signature: "floor(x)",
		summary: "Rounds down.",
	},
	Function {
		name: "ceil",
		signature: "ceil(x)",
		summary: "Rounds up.",
	},
	Function {
		name: "abs",
		signature: "abs(x)",
		summary: "The value without its sign.",
	},
	Function {
		name: "clamp",
		signature: "clamp(x, low, high)",
		summary: "Keeps `x` between `low` and `high`.",
	},
	Function {
		name: "pow",
		signature: "pow(x, power)",
		summary: "`x` raised to a power.",
	},
	Function {
		name: "if",
		signature: "if(condition, then, otherwise)",
		summary: "`then` when the condition holds, else `otherwise`; the same as `condition ? \
		          then : otherwise`.",
	},
	Function {
		name: "money",
		signature: "money(x, symbol = \"$\")",
		summary: "Formats an amount: `money(1800)` is `$1,800`, `money(42.5, \"€\")` is `€42.50`.",
	},
	Function {
		name: "percent",
		signature: "percent(fraction, digits = 0)",
		summary: "Formats a fraction as a percentage: `percent(0.25)` is `25%`.",
	},
	Function {
		name: "format",
		signature: "format(x, digits = 0)",
		summary: "Formats a number with thousands separators: `format(1234.5, 1)` is `1,234.5`.",
	},
	Function {
		name: "text",
		signature: "text(value)",
		summary: "The value as text.",
	},
	Function {
		name: "number",
		signature: "number(value)",
		summary: "Reads a number from text such as `\"1,800\"`.",
	},
	Function {
		name: "range",
		signature: "range(count) or range(start, end)",
		summary: "Whole numbers: `range(3)` is `[0, 1, 2]`.",
	},
	Function {
		name: "repeat",
		signature: "repeat(value, count)",
		summary: "A list of `count` copies of `value`.",
	},
	Function {
		name: "cumsum",
		signature: "cumsum(list)",
		summary: "Running totals: `cumsum([1, 2, 3])` is `[1, 3, 6]`.",
	},
	Function {
		name: "growth",
		signature: "growth(start, rate, count)",
		summary: "Compounding values: `growth(1000, 0.05, 3)` is `[1000, 1050, 1102.5]`.",
	},
	Function {
		name: "get",
		signature: "get(list or record, index or key)",
		summary: "One item, the same as `list[index]` or `record.key`.",
	},
	Function {
		name: "pluck",
		signature: "pluck(list of records, \"field\")",
		summary: "One field from every record.",
	},
	Function {
		name: "contains",
		signature: "contains(list or text, value)",
		summary: "Whether the list holds the value, or the text includes it.",
	},
	Function {
		name: "join",
		signature: "join(list, separator = \", \")",
		summary: "The items as one text.",
	},
	Function {
		name: "reverse",
		signature: "reverse(list)",
		summary: "The list back to front.",
	},
	Function {
		name: "slice",
		signature: "slice(list, start, end)",
		summary: "The items from `start` up to, not including, `end`.",
	},
];

struct Arguments {
	name: &'static str,
	values: Vec<Value>,
}

impl Arguments {
	fn count(&self, min: usize, max: usize) -> Result<(), FormulaError> {
		let count = self.values.len();
		if count < min || count > max {
			let expected = if min == max {
				format!("{min}")
			} else {
				format!("{min} to {max}")
			};
			return error(format!(
				"`{}` takes {expected} arguments, not {count}",
				self.name
			));
		}

		Ok(())
	}

	fn value(&self, index: usize) -> &Value {
		self.values.get(index).unwrap_or(&Value::Null)
	}

	fn number(&self, index: usize) -> Result<f64, FormulaError> {
		number(self.value(index), &format!("`{}`", self.name))
	}

	fn number_or(&self, index: usize, default: f64) -> Result<f64, FormulaError> {
		if self.values.len() > index {
			self.number(index)
		} else {
			Ok(default)
		}
	}

	/// The numbers in a list argument, or the arguments themselves when there
	/// is more than one.
	fn numbers(&self) -> Result<Vec<f64>, FormulaError> {
		let items: Vec<&Value> = match self.values.as_slice() {
			[Value::List(items)] => items.iter().collect(),
			values => values.iter().collect(),
		};

		items
			.into_iter()
			.map(|item| number(item, &format!("`{}`", self.name)))
			.collect()
	}

	fn list(&self, index: usize) -> Result<&[Value], FormulaError> {
		match self.value(index) {
			Value::List(items) => Ok(items),
			other => {
				error(format!(
					"`{}` needs a list, not {}",
					self.name,
					describe(other)
				))
			}
		}
	}

	fn count_argument(&self, index: usize, budget: &Budget) -> Result<usize, FormulaError> {
		let count = self.number(index)?;
		if !(0.0..=budget.max_list as f64).contains(&count) {
			return error(format!(
				"`{}` makes at most {} items",
				self.name, budget.max_list
			));
		}

		Ok(count as usize)
	}
}

fn digits(value: f64) -> Result<usize, FormulaError> {
	if (0.0..=6.0).contains(&value) {
		Ok(value as usize)
	} else {
		error("digits must be between 0 and 6")
	}
}

fn call(name: &str, values: Vec<Value>, budget: &mut Budget) -> Result<Value, FormulaError> {
	let Some(function) = FUNCTIONS.iter().find(|function| function.name == name) else {
		return error(format!("there is no function `{name}`"));
	};
	let arguments = Arguments {
		name: function.name,
		values,
	};

	match function.name {
		"sum" | "avg" | "min" | "max" => aggregate(&arguments),
		"len" => {
			arguments.count(1, 1)?;
			match arguments.value(0) {
				Value::List(items) => Ok(Value::Number(items.len() as f64)),
				Value::Text(text) => Ok(Value::Number(text.chars().count() as f64)),
				other => {
					error(format!(
						"`len` needs a list or text, not {}",
						describe(other)
					))
				}
			}
		}
		"round" | "floor" | "ceil" | "abs" | "clamp" | "pow" => math(&arguments),
		"if" => {
			arguments.count(3, 3)?;
			let index = if arguments.value(0).truthy() { 1 } else { 2 };
			Ok(arguments.value(index).clone())
		}
		"money" | "percent" | "format" | "text" | "number" => formatting(&arguments),
		_ => lists(&arguments, budget),
	}
}

fn aggregate(arguments: &Arguments) -> Result<Value, FormulaError> {
	let numbers = arguments.numbers()?;

	Ok(match arguments.name {
		"sum" => Value::Number(numbers.iter().sum()),
		"avg" if numbers.is_empty() => Value::Null,
		"avg" => Value::Number(numbers.iter().sum::<f64>() / numbers.len() as f64),
		"min" => {
			numbers
				.into_iter()
				.reduce(f64::min)
				.map_or(Value::Null, Value::Number)
		}
		_ => {
			numbers
				.into_iter()
				.reduce(f64::max)
				.map_or(Value::Null, Value::Number)
		}
	})
}

fn math(arguments: &Arguments) -> Result<Value, FormulaError> {
	let x = || arguments.number(0);

	let result = match arguments.name {
		"round" => {
			arguments.count(1, 2)?;
			let scale = 10_f64.powi(digits(arguments.number_or(1, 0.0)?)? as i32);
			(x()? * scale).round() / scale
		}
		"floor" => {
			arguments.count(1, 1)?;
			x()?.floor()
		}
		"ceil" => {
			arguments.count(1, 1)?;
			x()?.ceil()
		}
		"abs" => {
			arguments.count(1, 1)?;
			x()?.abs()
		}
		"clamp" => {
			arguments.count(3, 3)?;
			x()?.max(arguments.number(1)?).min(arguments.number(2)?)
		}
		_ => {
			arguments.count(2, 2)?;
			x()?.powf(arguments.number(1)?)
		}
	};

	Ok(if result.is_finite() {
		Value::Number(result)
	} else {
		Value::Null
	})
}

fn formatting(arguments: &Arguments) -> Result<Value, FormulaError> {
	match arguments.name {
		"money" => {
			arguments.count(1, 2)?;
			let symbol = match arguments.value(1) {
				Value::Null => "$".to_owned(),
				other => other.to_string(),
			};
			Ok(Value::Text(money(arguments.number(0)?, &symbol)))
		}
		"percent" => {
			arguments.count(1, 2)?;
			let places = digits(arguments.number_or(1, 0.0)?)?;
			Ok(Value::Text(format!(
				"{}%",
				grouped(arguments.number(0)? * 100.0, places)
			)))
		}
		"format" => {
			arguments.count(1, 2)?;
			let places = digits(arguments.number_or(1, 0.0)?)?;
			Ok(Value::Text(grouped(arguments.number(0)?, places)))
		}
		"text" => {
			arguments.count(1, 1)?;
			Ok(Value::Text(arguments.value(0).to_string()))
		}
		_ => {
			arguments.count(1, 1)?;
			Ok(arguments
				.value(0)
				.as_number()
				.map_or(Value::Null, Value::Number))
		}
	}
}

/// `number` with `places` decimals and thousands separators.
pub(crate) fn grouped(number: f64, places: usize) -> String {
	if !number.is_finite() {
		return "—".to_owned();
	}

	let fixed = format!("{:.*}", places, number.abs());
	let (whole, fraction) = fixed.split_once('.').unwrap_or((&fixed, ""));
	let sign = if number < 0.0
		&& fixed
			.chars()
			.any(|digit| digit.is_ascii_digit() && digit != '0')
	{
		"-"
	} else {
		""
	};

	if fraction.is_empty() {
		format!("{sign}{}", group_thousands(whole))
	} else {
		format!("{sign}{}.{fraction}", group_thousands(whole))
	}
}

/// An amount with a currency symbol, showing cents only when there are some.
pub(crate) fn money(amount: f64, symbol: &str) -> String {
	let cents = (amount * 100.0).round();
	let places = if cents % 100.0 == 0.0 { 0 } else { 2 };
	let text = grouped(cents.abs() / 100.0, places);

	if cents < 0.0 {
		format!("-{symbol}{text}")
	} else {
		format!("{symbol}{text}")
	}
}

fn lists(arguments: &Arguments, budget: &mut Budget) -> Result<Value, FormulaError> {
	match arguments.name {
		"range" => {
			arguments.count(1, 2)?;
			let (start, end) = if arguments.values.len() == 1 {
				(0.0, arguments.number(0)?)
			} else {
				(arguments.number(0)?, arguments.number(1)?)
			};
			let count = (end - start).max(0.0).ceil();
			if count > budget.max_list as f64 {
				return error(format!("`range` makes at most {} items", budget.max_list));
			}
			budget.list(
				(0..count as usize)
					.map(|offset| Value::Number(start + offset as f64))
					.collect(),
			)
		}
		"repeat" => {
			arguments.count(2, 2)?;
			let count = arguments.count_argument(1, budget)?;
			budget.list(vec![arguments.value(0).clone(); count])
		}
		"cumsum" => {
			arguments.count(1, 1)?;
			let mut total = 0.0;
			let items = arguments
				.numbers()?
				.into_iter()
				.map(|number| {
					total += number;
					Value::Number(total)
				})
				.collect();
			budget.list(items)
		}
		"growth" => {
			arguments.count(3, 3)?;
			let start = arguments.number(0)?;
			let rate = arguments.number(1)?;
			let count = arguments.count_argument(2, budget)?;
			let mut value = start;
			let items = (0..count)
				.map(|_| {
					let current = value;
					value *= 1.0 + rate;
					Value::Number(current)
				})
				.collect();
			budget.list(items)
		}
		_ => more_lists(arguments, budget),
	}
}

fn more_lists(arguments: &Arguments, budget: &mut Budget) -> Result<Value, FormulaError> {
	match arguments.name {
		"get" => {
			arguments.count(2, 2)?;
			get(arguments.value(0), arguments.value(1))
		}
		"pluck" => {
			arguments.count(2, 2)?;
			let field = arguments.value(1).to_string();
			let items = arguments
				.list(0)?
				.iter()
				.map(|item| {
					match item {
						Value::Record(fields) => fields.get(&field).cloned().unwrap_or_default(),
						_ => Value::Null,
					}
				})
				.collect();
			budget.list(items)
		}
		"contains" => {
			arguments.count(2, 2)?;
			Ok(Value::Bool(match arguments.value(0) {
				Value::List(items) => items.iter().any(|item| equal(item, arguments.value(1))),
				Value::Text(text) => text.contains(&arguments.value(1).to_string()),
				Value::Record(fields) => fields.contains_key(&arguments.value(1).to_string()),
				_ => false,
			}))
		}
		"join" => {
			arguments.count(1, 2)?;
			let separator = match arguments.value(1) {
				Value::Null => ", ".to_owned(),
				other => other.to_string(),
			};
			let parts: Vec<String> = arguments.list(0)?.iter().map(ToString::to_string).collect();
			Ok(Value::Text(parts.join(&separator)))
		}
		"reverse" => {
			arguments.count(1, 1)?;
			budget.list(arguments.list(0)?.iter().rev().cloned().collect())
		}
		_ => {
			arguments.count(2, 3)?;
			let items = arguments.list(0)?;
			let start = (arguments.number(1)?.max(0.0) as usize).min(items.len());
			let end = (arguments.number_or(2, items.len() as f64)?.max(0.0) as usize)
				.clamp(start, items.len());
			budget.list(items[start..end].to_vec())
		}
	}
}

/// A scope of fixed values, for tests.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Fixed(pub(crate) std::collections::BTreeMap<String, Value>);

#[cfg(test)]
impl Scope for Fixed {
	fn variable(&self, name: &str, _budget: &mut Budget) -> Result<Option<Value>, FormulaError> {
		Ok(self.0.get(name).cloned())
	}
}

/// Formats a value for display inside prose.
pub(crate) fn display(value: &Value) -> String {
	match value {
		Value::Number(number) => format_number(*number),
		other => other.to_string(),
	}
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use super::*;

	fn eval(source: &str) -> Result<Value, FormulaError> {
		let mut scope = Fixed::default();
		scope.0.insert("rent".to_owned(), Value::Number(1800.0));
		scope
			.0
			.insert("names".to_owned(), Value::from(vec!["a", "b"]));
		let mut fields = BTreeMap::new();
		fields.insert("groceries".to_owned(), Value::Number(600.0));
		scope.0.insert("budget".to_owned(), Value::Record(fields));
		Expr::parse(source)?.eval(&scope, &mut Budget::new(10_000, 1000))
	}

	#[test]
	fn evaluates_arithmetic_with_precedence() {
		assert_eq!(eval("1 + 2 * 3").unwrap(), Value::Number(7.0));
		assert_eq!(eval("(1 + 2) * 3").unwrap(), Value::Number(9.0));
		assert_eq!(eval("rent * 12").unwrap(), Value::Number(21600.0));
		assert_eq!(eval("-rent + 1").unwrap(), Value::Number(-1799.0));
		assert_eq!(eval("10 % 4").unwrap(), Value::Number(2.0));
		assert_eq!(eval("1 / 0").unwrap(), Value::Null);
	}

	#[test]
	fn evaluates_conditions() {
		assert_eq!(
			eval("rent > 1000 ? \"high\" : \"low\"").unwrap(),
			Value::from("high")
		);
		assert_eq!(
			eval("rent >= 1800 && rent <= 1800").unwrap(),
			Value::Bool(true)
		);
		assert_eq!(eval("!false || missing").unwrap(), Value::Bool(true));
		assert_eq!(eval("if(rent == 1800, 1, 2)").unwrap(), Value::Number(1.0));
	}

	#[test]
	fn reads_records_and_lists() {
		assert_eq!(eval("budget.groceries").unwrap(), Value::Number(600.0));
		assert_eq!(eval("names[1]").unwrap(), Value::from("b"));
		assert_eq!(eval("names.length").unwrap(), Value::Number(2.0));
		assert_eq!(eval("budget[\"groceries\"]").unwrap(), Value::Number(600.0));
		assert_eq!(eval("names[9]").unwrap(), Value::Null);
	}

	#[test]
	fn combines_lists_item_by_item() {
		assert_eq!(eval("[1, 2] * 12").unwrap(), Value::from(vec![12.0, 24.0]));
		assert_eq!(
			eval("[1, 2] + [3, 4]").unwrap(),
			Value::from(vec![4.0, 6.0])
		);
		assert_eq!(
			eval("cumsum(repeat(100, 3)) + 1000").unwrap(),
			Value::from(vec![1100.0, 1200.0, 1300.0])
		);
		assert!(eval("[1, 2] + [3]").is_err());
	}

	#[test]
	fn formats_amounts() {
		assert_eq!(eval("money(1800)").unwrap(), Value::from("$1,800"));
		assert_eq!(eval("money(42.5, \"€\")").unwrap(), Value::from("€42.50"));
		assert_eq!(
			eval("money(-1234567.891)").unwrap(),
			Value::from("-$1,234,567.89")
		);
		assert_eq!(eval("percent(0.256)").unwrap(), Value::from("26%"));
		assert_eq!(eval("percent(0.256, 1)").unwrap(), Value::from("25.6%"));
		assert_eq!(eval("format(1234.5, 1)").unwrap(), Value::from("1,234.5"));
		assert_eq!(
			eval("\"Rent: \" + money(rent)").unwrap(),
			Value::from("Rent: $1,800")
		);
	}

	#[test]
	fn aggregates() {
		assert_eq!(eval("sum([1, 2, 3])").unwrap(), Value::Number(6.0));
		assert_eq!(eval("avg([1, 2, 3])").unwrap(), Value::Number(2.0));
		assert_eq!(eval("max(1, 5, 3)").unwrap(), Value::Number(5.0));
		assert_eq!(eval("min([])").unwrap(), Value::Null);
		assert_eq!(
			eval("growth(1000, 0.05, 3)").unwrap(),
			Value::from(vec![1000.0, 1050.0, 1102.5])
		);
	}

	#[test]
	fn rejects_what_it_cannot_do() {
		assert!(eval("missing").is_err());
		assert!(eval("fetch(\"https://example.com\")").is_err());
		assert!(eval("rent = 3").is_err());
		assert!(eval("\"unterminated").is_err());
		assert!(eval(&"(".repeat(100)).is_err());
		assert!(eval("range(1000000)").is_err());
	}

	#[test]
	fn runs_out_of_fuel_instead_of_running_forever() {
		let scope = Fixed::default();
		let expr = Expr::parse("sum(range(1000)) + sum(range(1000))").unwrap();
		assert!(expr.eval(&scope, &mut Budget::new(500, 1000)).is_err());
		assert!(expr.eval(&scope, &mut Budget::new(10_000, 1000)).is_ok());
	}

	#[test]
	fn knows_constant_formulas() {
		assert!(Expr::parse("[1, 2] * 3").unwrap().is_constant());
		assert!(!Expr::parse("rent * 3").unwrap().is_constant());
		assert_eq!(Expr::parse("rent").unwrap().as_variable(), Some("rent"));
	}
}
