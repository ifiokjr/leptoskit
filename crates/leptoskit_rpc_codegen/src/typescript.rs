//! The TypeScript client: one dependency-free module over `fetch`.
//!
//! Everything lands in a single `index.ts` so the package works unchanged
//! under Deno, Bun, Node with a TypeScript loader, or a bundler — there are no
//! relative imports whose extension rules differ between them. Types are
//! structural translations of the JSON Schemas; requests are encoded exactly
//! as leptos's `serde_qs` and JSON codecs expect, and error bodies are parsed
//! into a `FnError` classified like `leptoskit_rpc::FnError`.

// Generators build source text; the allocation `write!` would save is noise here.
#![allow(clippy::format_push_string)]

use std::collections::HashMap;

use serde_json::Value;

use crate::generate::Context;
use crate::generate::Generated;
use crate::generate::GeneratedFile;
use crate::naming::Scope;
use crate::naming::TS_RESERVED_METHODS;
use crate::naming::TS_RESERVED_TYPES;
use crate::naming::component_names;
use crate::naming::ts_method;
use crate::naming::ts_property_key;
use crate::naming::ts_string;
use crate::schema::Shape;
use crate::schema::analyze;
use crate::schema::description;
use crate::schema::has_default;
use crate::spec::ApiFunction;
use crate::spec::HttpInput;

pub(crate) fn generate(context: &Context<'_>) -> Generated {
	let names = component_names(context.spec, &mut Scope::new(TS_RESERVED_TYPES));
	let methods = method_names(context);
	let mut index = context.banner("//");
	index.push_str(&types_section(context, &names));
	index.push_str(RUNTIME);
	index.push_str(&client_section(context, &names, &methods));
	Generated {
		files: vec![
			GeneratedFile::code("index.ts", index),
			GeneratedFile::scaffold("package.json", package_json(context)),
			GeneratedFile::scaffold("tsconfig.json", TSCONFIG.to_owned()),
			GeneratedFile::scaffold("README.md", readme(context, &methods)),
		],
	}
}

/// The method name of every callable function, in order, clear of
/// `BaseClient`'s members and of each other.
fn method_names(context: &Context<'_>) -> Vec<String> {
	let mut scope = Scope::new(TS_RESERVED_METHODS);
	context
		.functions()
		.map(|function| scope.bind(&ts_method(&function.name)))
		.collect()
}

/// `export interface`/`export type` for every component schema, under the
/// names in `names`.
fn types_section(context: &Context<'_>, names: &HashMap<String, String>) -> String {
	let mut out = String::new();

	for (name, schema) in &context.spec.schemas {
		let ts_name = &names[name];
		push_jsdoc(&mut out, description(schema), "");
		let analyzed = analyze(schema);

		match (&analyzed.shape, analyzed.nullable) {
			(Shape::Object(object), false) if !object.properties.is_empty() => {
				out.push_str(&format!("export interface {ts_name} {{\n"));
				for (property, property_schema) in &object.properties {
					push_jsdoc(&mut out, description(property_schema), "\t");
					let optional = !object.is_required(property) || has_default(property_schema);
					out.push_str(&format!(
						"\t{}{}: {};\n",
						ts_property_key(property),
						if optional { "?" } else { "" },
						render(property_schema, names)
					));
				}

				out.push_str("}\n\n");
			}

			_ => {
				out.push_str(&format!(
					"export type {ts_name} = {};\n\n",
					render(schema, names)
				));
			}
		}
	}

	out
}

/// A TypeScript type expression for a schema, naming components as `names`
/// does. References to schemas the spec does not carry render as `unknown`
/// rather than as a name nothing defines.
fn render(schema: &Value, names: &HashMap<String, String>) -> String {
	let analyzed = analyze(schema);

	let base = match analyzed.shape {
		Shape::Ref(name) => {
			names
				.get(name)
				.cloned()
				.unwrap_or_else(|| "unknown".to_owned())
		}
		Shape::Any => "unknown".to_owned(),
		Shape::Null => "null".to_owned(),
		Shape::Boolean => "boolean".to_owned(),
		Shape::Integer { .. } | Shape::Number { .. } => "number".to_owned(),
		Shape::String {
			values: Some(values),
			..
		} => {
			values
				.iter()
				.map(|value| ts_string(value))
				.collect::<Vec<_>>()
				.join(" | ")
		}
		Shape::String {
			constant: Some(constant),
			..
		} => ts_string(constant),
		Shape::String { .. } => "string".to_owned(),
		Shape::Array { items } => {
			let inner = items.map_or_else(|| "unknown".to_owned(), |items| render(items, names));
			if inner.contains(' ') {
				format!("({inner})[]")
			} else {
				format!("{inner}[]")
			}
		}
		Shape::Tuple(items) => {
			format!(
				"[{}]",
				items
					.iter()
					.map(|item| render(item, names))
					.collect::<Vec<_>>()
					.join(", ")
			)
		}
		Shape::Object(object) => {
			if object.properties.is_empty() {
				match object.additional {
					Some(values) => format!("Record<string, {}>", render(values, names)),
					None => "Record<string, never>".to_owned(),
				}
			} else {
				let fields: Vec<String> = object
					.properties
					.iter()
					.map(|(property, property_schema)| {
						let optional =
							!object.is_required(property) || has_default(property_schema);
						format!(
							"{}{}: {}",
							ts_property_key(property),
							if optional { "?" } else { "" },
							render(property_schema, names)
						)
					})
					.collect();
				format!("{{ {} }}", fields.join("; "))
			}
		}
		Shape::Map { values } => {
			format!(
				"Record<string, {}>",
				values.map_or_else(|| "unknown".to_owned(), |values| render(values, names))
			)
		}
		Shape::Union(variants) => {
			variants
				.iter()
				.map(|variant| render(variant, names))
				.collect::<Vec<_>>()
				.join(" | ")
		}
	};

	if analyzed.nullable && base != "null" {
		format!("{base} | null")
	} else {
		base
	}
}

/// The type of a function's input object, or `None` for no arguments.
fn input_type(function: &ApiFunction, names: &HashMap<String, String>) -> Option<String> {
	if function.params.is_empty() {
		return None;
	}

	let fields: Vec<String> = function
		.params
		.iter()
		.map(|param| {
			// The argument's doc, from its doc comment or the `# Arguments`
			// section, escaped like every other doc block.
			let mut field = String::new();
			push_jsdoc(&mut field, Some(&param.docs), "\t");
			field.push_str(&format!(
				"\t{}{}: {}",
				ts_property_key(&param.name),
				if param.is_required() { "" } else { "?" },
				render(&param.schema, names)
			));
			field
		})
		.collect();
	Some(format!("{{\n{}\n}}", fields.join(";\n")))
}

fn client_section(
	context: &Context<'_>,
	names: &HashMap<String, String>,
	methods: &[String],
) -> String {
	let mut out = String::new();
	out.push_str(&format!(
		"/** The server origin baked in at generation time; `undefined` means the page's own \
		 origin. */\nexport const DEFAULT_SERVER_URL: string | undefined = {};\n\n",
		context
			.server_url
			.as_deref()
			.map_or_else(|| "undefined".to_owned(), ts_string)
	));
	push_jsdoc(
		&mut out,
		Some(&format!(
			"A typed client for the {} RPC, version {}.",
			context.spec.title, context.spec.version
		)),
		"",
	);
	out.push_str("export class Client extends BaseClient {\n");

	for (function, method) in context.functions().zip(methods) {
		out.push('\n');
		let mut docs = function.docs.clone();
		push_arguments(&mut docs, function);

		if function.deprecated {
			docs.push_str(if docs.is_empty() {
				"@deprecated"
			} else {
				"\n\n@deprecated"
			});
		}

		push_jsdoc(&mut out, (!docs.is_empty()).then_some(docs.as_str()), "\t");
		let output = render(&function.output_schema, names);
		let encoding = encoding(function);
		// A mutation takes an idempotency key, so a retried call runs once.
		let (options, key) = if function.kind.is_idempotent_with_key() {
			("options?: CallOptions", ", options?.idempotencyKey")
		} else {
			("", "")
		};

		match input_type(function, names) {
			Some(input) => {
				let separator = if options.is_empty() { "" } else { ", " };
				out.push_str(&format!(
					"\t{method}(input: {}{separator}{options}): Promise<{output}> {{\n\t\treturn \
					 this.call<{output}>({}, {}, {}, input{key});\n\t}}\n",
					input.replace('\n', "\n\t"),
					ts_string(&function.method),
					ts_string(&function.path),
					ts_string(encoding),
				));
			}
			None => {
				out.push_str(&format!(
					"\t{method}({options}): Promise<{output}> {{\n\t\treturn \
					 this.call<{output}>({}, {}, {}, {{}}{key});\n\t}}\n",
					ts_string(&function.method),
					ts_string(&function.path),
					ts_string(encoding),
				));
			}
		}
	}

	out.push_str("}\n\n");
	out.push_str(
		"/** A client with the default options; construct `new Client(...)` for anything else. \
		 */\nexport const client = new Client();\n",
	);
	out
}

/// Appends an `# Arguments`-style block listing each argument's doc.
pub(crate) fn push_arguments(docs: &mut String, function: &ApiFunction) {
	let documented: Vec<&crate::spec::ApiParam> = function
		.params
		.iter()
		.filter(|param| !param.docs.is_empty())
		.collect();

	if documented.is_empty() {
		return;
	}

	docs.push_str("\n\n");

	for param in documented {
		docs.push_str(&format!("- `{}` — {}\n", param.name, param.docs.trim()));
	}

	docs.pop();
}

pub(crate) fn encoding(function: &ApiFunction) -> &'static str {
	match function.input {
		HttpInput::Query => "query",
		HttpInput::Form => "form",
		HttpInput::Json | HttpInput::Other { .. } => "json",
	}
}

/// Pushes a `JSDoc` block at `indent`.
pub(crate) fn push_jsdoc(out: &mut String, docs: Option<&str>, indent: &str) {
	let Some(docs) = docs else {
		return;
	};
	let docs = docs.trim();

	if docs.is_empty() {
		return;
	}

	let safe = docs.replace("*/", "*\\/");
	let lines: Vec<&str> = safe.lines().collect();

	if lines.len() == 1 && !lines[0].starts_with("- ") {
		out.push_str(&format!("{indent}/** {} */\n", lines[0]));
	} else {
		out.push_str(&format!("{indent}/**\n"));

		for line in lines {
			if line.is_empty() {
				out.push_str(&format!("{indent} *\n"));
			} else {
				out.push_str(&format!("{indent} * {line}\n"));
			}
		}

		out.push_str(&format!("{indent} */\n"));
	}
}

fn package_json(context: &Context<'_>) -> String {
	let value = serde_json::json!({
		"name": context.options.package_name,
		"version": context.version,
		"description": format!("A typed client for the {} RPC.", context.spec.title),
		"type": "module",
		"exports": "./index.ts",
		"types": "./index.ts",
		"files": ["index.ts", "README.md"],
		"sideEffects": false,
	});
	serde_json::to_string_pretty(&value).unwrap_or_default() + "\n"
}

fn readme(context: &Context<'_>, methods: &[String]) -> String {
	let example =
		context
			.functions()
			.zip(methods)
			.next()
			.map_or_else(String::new, |(function, method)| {
				let args = if function.params.is_empty() {
					String::new()
				} else {
					let fields: Vec<String> = function
						.params
						.iter()
						.filter(|param| param.is_required())
						.map(|param| format!("{}: /* ... */", ts_property_key(&param.name)))
						.collect();
					format!("{{ {} }}", fields.join(", "))
				};

				format!("const result = await api.{method}({args});\n")
			});
	let mut skipped = String::new();

	if !context.skipped.is_empty() {
		skipped.push_str("\n## Not generated\n\n");

		for (name, reason) in &context.skipped {
			skipped.push_str(&format!("- `{name}`: {reason}\n"));
		}
	}

	format!(
		"# {name}

A typed TypeScript client for the **{title} {version}** RPC, generated by `leptoskit rpc generate \
		 --target typescript`.

It has no dependencies and uses the platform `fetch`; the calls go to the same routes the app \
		 itself uses. The package ships TypeScript source (`index.ts`), which Deno, Bun, and \
		 TypeScript-aware bundlers load directly; for plain Node, compile it with `npx tsc -p .`.

## Usage

```ts
import {{ Client, FnError }} from {import_name};

const api = new Client({{ serverUrl: {server_url} }});

try {{
	{example}}} catch (error) {{
	if (error instanceof FnError && error.retryable) {{
		// a network or server failure: safe to try again
	}}
	throw error;
}}
```

`Client` takes `serverUrl` (defaults to the page's own origin), `headers` sent with every request \
		 (for example `Authorization`), and a `fetch` implementation.

Errors are `FnError` instances with a `kind` of `network`, `invalid`, `server`, `decode`, or \
		 `other`, mirroring the server's classification; `retryable` is true only for \
		 `network`.{skipped}",
		name = context.options.package_name,
		title = context.spec.title,
		version = context.spec.version,
		import_name = ts_string(&context.options.package_name),
		server_url = ts_string(
			context
				.server_url
				.as_deref()
				.unwrap_or("https://example.com")
		),
	)
}

const TSCONFIG: &str = r#"{
	"compilerOptions": {
		"target": "ES2022",
		"module": "ESNext",
		"moduleResolution": "Bundler",
		"lib": ["ES2022", "DOM"],
		"strict": true,
		"declaration": true,
		"outDir": "dist",
		"skipLibCheck": true
	},
	"include": ["index.ts"]
}
"#;

/// The runtime every generated TypeScript client shares.
const RUNTIME: &str = r#"/** How a failed call is classified, mirroring `leptoskit_rpc::FnError`. */
export type FnErrorKind = "network" | "invalid" | "server" | "decode" | "other";

/** A failed RPC call. */
export class FnError extends Error {
	/** The classification. */
	readonly kind: FnErrorKind;
	/** The HTTP status, when a response arrived. */
	readonly status: number | undefined;
	/** The raw response body, when a response arrived. */
	readonly raw: string | undefined;

	constructor(
		kind: FnErrorKind,
		message: string,
		options: { status?: number; raw?: string; cause?: unknown } = {},
	) {
		super(message, options.cause === undefined ? undefined : { cause: options.cause });
		this.name = "FnError";
		this.kind = kind;
		this.status = options.status;
		this.raw = options.raw;
	}

	/**
	 * Whether retrying the call could plausibly succeed: only when no answer
	 * arrived. Server errors are final, because the server reports a refusal
	 * such as "not found" the same way as an outage.
	 */
	get retryable(): boolean {
		return this.kind === "network";
	}

	/**
	 * Parses leptos's error body: `Kind|message` as `text/plain`, where `Kind`
	 * is one of the `ServerFnError` variants.
	 */
	static fromResponse(status: number, body: string): FnError {
		const separator = body.indexOf("|");
		if (separator === -1) {
			return new FnError("other", body || `HTTP ${status}`, { status, raw: body });
		}
		const variant = body.slice(0, separator);
		const message = body.slice(separator + 1);
		let kind: FnErrorKind;
		switch (variant) {
			case "Request":
				kind = "network";
				break;
			case "Args":
			case "MissingArg":
				kind = "invalid";
				break;
			case "ServerError":
			case "MiddlewareError":
			case "WrappedServerFn":
				kind = "server";
				break;
			case "Deserialization":
			case "Serialization":
				kind = "decode";
				break;
			default:
				kind = "other";
		}
		return new FnError(kind, message, { status, raw: body });
	}
}

/** Options for a `Client`. */
export interface ClientOptions {
	/** The origin the RPC is served from, without a trailing slash. Defaults to the page's own origin. */
	serverUrl?: string;
	/** Headers sent with every request, such as `Authorization`. */
	headers?: Record<string, string>;
	/** The `fetch` implementation to use; defaults to the global one. */
	fetch?: typeof fetch;
}

/** Options for one call. */
export interface CallOptions {
	/**
	 * Any unique string, such as `crypto.randomUUID()`. Sending a mutation again
	 * with the same key and arguments returns the first call's result instead of
	 * running it twice, so a call whose answer was lost is safe to retry.
	 */
	idempotencyKey?: string;
}

/** How a request body is encoded; chosen per function by the server's codec. */
type Encoding = "query" | "form" | "json";

/**
 * Encodes a value the way `serde_qs` does: nested keys in brackets, arrays by
 * index, `null`/`undefined` omitted.
 */
export function toQueryString(value: Record<string, unknown>): string {
	const params = new URLSearchParams();
	const append = (key: string, item: unknown): void => {
		if (item === null || item === undefined) {
			return;
		}
		if (Array.isArray(item)) {
			item.forEach((entry, index) => append(`${key}[${index}]`, entry));
		} else if (typeof item === "object") {
			for (const [field, entry] of Object.entries(item as Record<string, unknown>)) {
				append(`${key}[${field}]`, entry);
			}
		} else {
			params.append(key, String(item));
		}
	};
	for (const [key, item] of Object.entries(value)) {
		append(key, item);
	}
	return params.toString();
}

/** The transport shared by every generated method. */
export class BaseClient {
	readonly serverUrl: string;
	readonly headers: Record<string, string>;
	readonly fetchImpl: typeof fetch;

	constructor(options: ClientOptions = {}) {
		this.serverUrl = (options.serverUrl ?? DEFAULT_SERVER_URL ?? "").replace(/\/+$/, "");
		this.headers = { ...(options.headers ?? {}) };
		this.fetchImpl = options.fetch ?? ((input, init) => globalThis.fetch(input, init));
	}

	protected async call<T>(
		method: string,
		path: string,
		encoding: Encoding,
		input: Record<string, unknown>,
		idempotencyKey?: string,
	): Promise<T> {
		let url = `${this.serverUrl}${path}`;
		const headers: Record<string, string> = { Accept: "application/json", ...this.headers };
		if (idempotencyKey !== undefined) {
			headers["Idempotency-Key"] = idempotencyKey;
		}
		let body: string | undefined;
		if (encoding === "query") {
			const query = toQueryString(input);
			if (query !== "") {
				url += (url.includes("?") ? "&" : "?") + query;
			}
		} else if (encoding === "form") {
			headers["Content-Type"] = "application/x-www-form-urlencoded";
			body = toQueryString(input);
		} else {
			headers["Content-Type"] = "application/json";
			// Absent optional arguments are omitted, as `serde_qs` omits them:
			// the server fills `#[server(default)]` arguments in, and `Option`
			// arguments read a missing key as `None`.
			const present = Object.fromEntries(
				Object.entries(input).filter(([, item]) => item !== undefined && item !== null),
			);
			body = JSON.stringify(present);
		}
		let response: Response;
		try {
			response = await this.fetchImpl(url, { method, headers, body });
		} catch (cause) {
			throw new FnError("network", `could not reach ${url}: ${String(cause)}`, { cause });
		}
		const text = await response.text();
		if (response.status >= 400) {
			throw FnError.fromResponse(response.status, text);
		}
		if (text === "") {
			return null as T;
		}
		try {
			return JSON.parse(text) as T;
		} catch (cause) {
			throw new FnError("decode", `the response was not JSON: ${String(cause)}`, {
				status: response.status,
				raw: text,
				cause,
			});
		}
	}
}

"#;
