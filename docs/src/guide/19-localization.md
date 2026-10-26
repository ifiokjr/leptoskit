# Localization

An app that speaks several languages needs more than a dictionary. "1 item" and "2 items" differ in English; Polish has four forms, Arabic six, and Japanese one. A message with a name in it is a method with a parameter, not a string with a hole. And when the user switches language, every piece of text on screen has to follow.

leptoskit handles this the way Flutter's `gen_l10n` does. Your messages live in one ARB file per locale, and `leptoskit l10n` writes `src/l10n.rs` from them: a `Messages` trait with one method per message, typed by the message's placeholders, implemented once per locale. A component reads its text through a reactive handle, so this is all a translated, pluralised, re-rendering label takes:

```rust
let l10n = use_l10n();
view! {
	<Text>{move || l10n.cart_items(count.get())}</Text>
}
```

A message that does not exist, a placeholder that is missing, or a number passed where a name belongs is a compile error, not a blank label in production. The module is ordinary Rust, so the same code runs on web, iOS, Android, macOS, Windows, Linux, and HarmonyOS (experimental); only where the user's preferred locales come from differs per platform, and that is covered below.

```text
my_app/
├── leptoskit.toml          [l10n]
├── locales/
│   ├── en.arb              the template: every message, with its placeholders
│   ├── pt-BR.arb           a translation; messages it leaves out fall back to en
│   └── ar.arb
└── src/
    ├── lib.rs              mod l10n;
    └── l10n.rs             generated, committed
```

## Set up translations

<Steps>

### Add the runtime crate

The generated module calls into `leptoskit_l10n`, so the app depends on it directly, the way it depends on `leptoskit_router` for file routes. Turn on its `web` feature in web builds to read the browser's languages.

```toml
[dependencies]
leptoskit_l10n = "0.1"

[features]
web = ["leptoskit/web", "leptoskit_l10n/web"]
```

The `leptoskit` meta crate also re-exports the runtime as `leptoskit::l10n` behind its `l10n` feature.

### Turn it on

An `[l10n]` table in `leptoskit.toml` enables message generation, even when it is empty. These are the defaults:

```toml
[l10n]
dir = "locales" # the ARB files, one per locale
output = "src/l10n.rs" # the generated module
template = "en" # the locale whose file defines every message
```

### Write the template

Create `locales/en.arb`. The template defines every message the app has and, through `@key` entries, what each placeholder is. Keys are written the way Flutter writes them, in camel case, and become snake case methods.

```json
{
	"@@locale": "en",
	"appTitle": "Field Notes",
	"greeting": "Hello, {name}!",
	"@greeting": {
		"description": "Shown at the top of the home screen.",
		"placeholders": {
			"name": { "type": "String", "example": "Ana" }
		}
	},
	"cartItems": "{count, plural, =0 {Your cart is empty} one {# item in your cart} other {# items in your cart}}",
	"@cartItems": {
		"placeholders": {
			"count": { "type": "int" }
		}
	},
	"pronoun": "{gender, select, female {she} male {he} other {they}}"
}
```

### Generate the module

```bash
leptoskit l10n
```

It writes `src/l10n.rs` and lists every locale with how complete it is, then every message with the method it became:

```text
wrote src/l10n.rs
   en     en.arb     template, 4 messages
   pt-BR  pt-BR.arb  3 of 4 translated; missing pronoun
   appTitle   app_title() -> &'static str
   cartItems  cart_items(count: i64) -> String
   greeting   greeting(name: &str) -> String
   pronoun    pronoun(gender: &str) -> String
```

Declare the module in `src/lib.rs` and commit it. A fresh clone compiles with plain `cargo build`, and rust-analyzer sees the methods and their documentation, which carries each message's text.

```rust
mod l10n;
```

`leptoskit dev`, `leptoskit build`, and native builds regenerate the module before compiling and write it only when the messages changed, so after editing an ARB file you only run `leptoskit l10n` yourself when you want to see the listing. In CI, fail when someone forgot to commit the result:

```bash
leptoskit l10n --check
```

### Provide the translations

Where the app starts, create an `L10n` from the locales the user prefers and provide it to the tree. `L10n::new` picks the supported locale that serves those preferences best and falls back to the template when none does.

```rust
use leptoskit::prelude::*;
use leptoskit_l10n::Locale;

use crate::l10n::L10n;

#[component]
pub fn App(preferred: Vec<Locale>) -> impl IntoView {
	L10n::new(&preferred).provide();
	view! { <Home /> }
}
```

The preferred locales come from the platform:

<Tabs>
<Tab label="Browser">

With the `web` feature, `browser_locales` reads `navigator.languages`, best first. Outside a browser it is empty, so shared code can call it everywhere.

```rust
let preferred = leptoskit_l10n::browser_locales();
```

</Tab>
<Tab label="Server">

A server-rendered page should come out in the language the browser will hydrate it in. `accept_language` turns the request's `Accept-Language` header into the same ordered list the browser would give; with `leptos_axum`, the request's parts are in context.

```rust
use leptoskit_l10n::accept_language;

let preferred = use_context::<http::request::Parts>()
	.and_then(|parts| {
		let header = parts.headers.get(http::header::ACCEPT_LANGUAGE)?;
		Some(accept_language(header.to_str().ok()?))
	})
	.unwrap_or_default();
```

</Tab>
<Tab label="Native">

The `localization` runtime plugin reports the device's languages in preference order, on iOS, Android, macOS, Windows, and Linux. Grant it in `leptoskit.toml` as described in [runtime plugins](../platforms/plugins.md), then parse the tags it returns.

```rust
use leptoskit_l10n::Locale;
use leptoskit_plugin_localization::guest::Localization;

let snapshot = Localization::new()?.snapshot().await?;
let preferred: Vec<Locale> = snapshot
	.locales
	.iter()
	.filter_map(|locale| Locale::new(&locale.tag).ok())
	.collect();
```

</Tab>
</Tabs>

An empty list starts the app on the template locale, which is also the right choice when the app remembers the user's own pick and sets it afterwards.

### Read messages in components

`use_l10n` returns the handle an ancestor provided. It is `Copy`, like a signal, and every message is a method on it. Reading a message inside a closure tracks the locale, so the view renders again when the locale changes.

```rust
use leptoskit::prelude::*;

use crate::l10n::use_l10n;

#[component]
pub fn Cart(count: RwSignal<i64>) -> impl IntoView {
	let l10n = use_l10n();
	view! {
		<View class="gap-2 p-4">
			<Text class="text-2xl font-bold">{l10n.app_title()}</Text>
			<Text>{move || l10n.greeting("Ana")}</Text>
			<Text>{move || l10n.cart_items(count.get())}</Text>
		</View>
	}
}
```

A message without placeholders returns `&'static str`; one with placeholders returns a `String`. Rust infers `3` as the `i64` that `cart_items` takes, so counts read as plainly as `l10n.cart_items(3)`.

### Let the user switch

`set_locale` switches to the supported locale that best serves the one given, and `supported_locales` lists every locale with a file, the template first: everything a language menu needs.

```rust
use leptoskit::prelude::*;

use crate::l10n::use_l10n;

#[component]
pub fn LanguageMenu() -> impl IntoView {
	let l10n = use_l10n();
	view! {
		<View class="flex-row gap-2">
			{l10n
				.supported_locales()
				.iter()
				.map(|locale| {
					let tag = locale.to_string();
					let choice = locale.clone();
					view! {
						<Button on:press=move || l10n.set_locale(&choice)>{tag}</Button>
					}
				})
				.collect::<Vec<_>>()}
		</View>
	}
}
```

To remember the choice, store the tag with the `store` runtime plugin or `leptoskit_use`'s storage hooks and pass it as the preferred locale at the next start.

</Steps>

## Writing messages

A message is text with placeholders, plurals, and selects, in the subset of ICU MessageFormat that ARB files use.

### Placeholders

`{name}` in a message becomes a parameter. Its type comes from the template's `@key` entry, or from how the message uses it:

| In `@key` → `placeholders` | Parameter | When it is the default                        |
| -------------------------- | --------- | --------------------------------------------- |
| `"type": "String"`         | `&str`    | a placeholder that is shown or matched        |
| `"type": "int"`            | `i64`     | a placeholder that is counted by a plural     |
| `"type": "num"`, `double`  | `f64`     | never; declare it for amounts with a fraction |

The `description` of an `@key` entry becomes the method's documentation, above the message's text. Other keys there, such as `example`, are for translators and translation tools; leptoskit leaves them alone. Parameters follow the order in which the template first uses them.

### Plurals

`{count, plural, ...}` picks a form by the number's CLDR plural category in the message's language. Name the forms the language has, `zero`, `one`, `two`, `few`, `many`, and always `other`; a number whose category has no form takes `other`. `=0`, `=1`, and so on match an exact number first, and `#` is the number itself.

```json
"cartItems": "{count, plural, =0 {Your cart is empty} one {# item in your cart} other {# items in your cart}}"
```

The template's `one` and `other` are English's two forms. A Polish translation writes `one`, `few`, `many`, and `other`, and gets each at the right numbers, because the generated code asks the rules of the translation's own locale, not the template's:

```json
"cartItems": "{count, plural, =0 {Koszyk jest pusty} one {# przedmiot w koszyku} few {# przedmioty w koszyku} many {# przedmiotów w koszyku} other {# przedmiotu w koszyku}}"
```

The rules are CLDR's, for every language it covers; `pt-BR` uses the `pt` rules while `pt-PT` has its own. Floats count by their shortest decimal form, so `1.0` is `one` in English and `1.5` is `other`.

### Selects

`{gender, select, ...}` picks a form by a word. Name the words the message distinguishes, and always `other`, for every value the message does not name. A select's placeholder is a `&str`.

```json
"pronoun": "{gender, select, female {she} male {he} other {they}}"
```

Plurals and selects nest: `{who, select, you {You have {count, plural, one {# message} other {# messages}}} other {{who} has {count, plural, one {# message} other {# messages}}}}` is one message with two parameters.

### Quoting

`''` is an apostrophe. A `'` before `{`, `}`, or `#` quotes literal text up to the next `'`, so `'{'` is a brace and `'#'` is a hash inside a plural. Any other `'` is itself: `don't` needs no care.

### Keys and methods

`cartItems` becomes `cart_items`; `URLLabel` becomes `url_label`. A key that would become a Rust keyword, such as `type`, or two keys that would become the same method, are errors that name the file and line. Messages are listed by method name in the module, so `appTitle` and `URLLabel` sit where `app_title` and `url_label` sort.

## Adding a language

Add `locales/<tag>.arb`, named after the locale's language tag: `pt-BR.arb`, `zh-Hant.arb`, `ar.arb`. The tag names a language, and optionally a script and a region, as BCP 47 writes them; `@@locale` inside the file is checked against the name. Translate the messages you have, and leave out the ones you do not: an untranslated message falls back to the template's text, and `leptoskit l10n` lists what each file is missing, so a translation can land in pieces without a blank label anywhere.

Every translation must use the template's placeholders, each in a way its type allows: a count stays a number, and a translation cannot invent `{surprise}`. A key that the template does not have is an error, so a message is always added to the template first.

### How a locale is chosen

`L10n::new` and `set_locale` resolve the user's preferences the way Flutter's `basicLocaleListResolution` does. For each preferred locale in turn, they look for a supported locale with the same tag; then one with the same language and script; then one with the same language and region; and last one with the same language alone. A language-only match is held back only while the next preferred locale has the same language, since that one might match a region exactly: `[en-GB, en-US]` picks `en-US` from `[en, en-US]`. When several supported locales share a language, one without a region stands in for the whole language ahead of the regional ones, so `pt-PT` gets `pt` from `[pt-BR, pt]`. When nothing matches, the template serves. `leptoskit_l10n::negotiate` is this step on its own, for code that wants to show the user what it chose.

## What the generated module holds

`src/l10n.rs` is ordinary Rust, formatted by your own rustfmt like any other file: formatting is not a change, so `leptoskit l10n --check` stays green after `leptoskit format`.

```rust
/// One method per message in `locales/en.arb`, typed by its placeholders.
pub trait Messages: Send + Sync {
	/// `Field Notes`
	fn app_title(&self) -> &'static str;
	/// `{count, plural, =0 {Your cart is empty} one {# item in your cart} other {# items in your cart}}`
	fn cart_items(&self, count: i64) -> String;
	/// Shown at the top of the home screen.
	///
	/// `Hello, {name}!`
	fn greeting(&self, name: &str) -> String;
}

/// The `pt-BR` messages, from `locales/pt-BR.arb`. Messages it leaves out
/// fall back to `En`: `pronoun`.
pub struct PtBr;

impl Messages for PtBr {
	fn app_title(&self) -> &'static str {
		"Notas de Campo"
	}

	fn cart_items(&self, count: i64) -> String {
		if count == 0 {
			"Seu carrinho está vazio".to_owned()
		} else if plural_pt_br(count) == PluralCategory::One {
			format!("{count} item no carrinho")
		} else {
			format!("{count} itens no carrinho")
		}
	}

	fn greeting(&self, name: &str) -> String {
		format!("Olá, {name}!")
	}

	fn pronoun(&self, gender: &str) -> String {
		En.pronoun(gender)
	}
}
```

Alongside these, the module declares `L10n`, the handle for this app's messages; `use_l10n`, which returns the one an ancestor provided; and `AppTranslations`, which tells the runtime which locale has which messages. leptoskit never overwrites an `l10n.rs` it did not write.

## Coming from Flutter

| Flutter                                          | leptoskit                                                |
| ------------------------------------------------ | -------------------------------------------------------- |
| `l10n.yaml`                                      | the `[l10n]` table in `leptoskit.toml`                   |
| `lib/l10n/app_en.arb`                            | `locales/en.arb`                                         |
| `flutter gen-l10n`                               | `leptoskit l10n`, also run by `dev` and `build`          |
| `AppLocalizations`                               | the generated `Messages` trait and `L10n` handle         |
| `AppLocalizations.of(context)!.cartItems(count)` | `use_l10n().cart_items(count)`                           |
| `supportedLocales`                               | `l10n.supported_locales()`, from the files in `locales/` |
| `basicLocaleListResolution`                      | `L10n::new(&preferred)`, `set_locale`, and `negotiate`   |
| `untranslated-messages-file`                     | the missing messages `leptoskit l10n` lists per locale   |
| `MaterialApp(locale: ...)`                       | `l10n.set_locale(&locale)`                               |

## What is not here yet

- Placeholders are strings and numbers. Dates, times, currencies, and formatted numbers are not typed or formatted; format them in Rust and pass the text as a `String` placeholder. `#` writes the number plainly, without grouping separators.
- Plurals are cardinal. Ordinal forms (`1st`, `2nd`) and `offset:` are rejected with an error that says so.
- The locale is not saved or carried from server to client for you. Pass the same preferences on both sides, as the server and browser tabs above do, and store a user's own choice with the storage of your choice.
