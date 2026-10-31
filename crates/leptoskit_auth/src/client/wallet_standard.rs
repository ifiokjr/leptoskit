//! Solana wallets in the browser, through the Wallet Standard
//! (`@wallet-standard/app`'s protocol, without its JavaScript): the page
//! announces itself with a `wallet-standard:app-ready` event, and wallets
//! register with `wallet-standard:register-wallet`, both carrying a
//! `register` callback. Wallets that registered earlier answer the first;
//! wallets that load later send the second.
//!
//! Signing prefers `solana:signIn`, where the wallet writes the Sign-In
//! With Solana message itself and shows the person the site and the
//! statement, and falls back to `standard:connect` plus
//! `solana:signMessage`, signing the text [`SiwsInput::message`] writes.
//! Either way the server checks the signed text byte for byte.

use std::cell::Cell;
use std::cell::RefCell;

use js_sys::Array;
use js_sys::Function;
use js_sys::Object;
use js_sys::Promise;
use js_sys::Reflect;
use js_sys::Uint8Array;
use leptoskit_db::Pubkey;
use leptoskit_db::Signature;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

use super::WalletApp;
use super::protocol::error;
use crate::ErrorResponse;
use crate::SiwsInput;
use crate::WalletProof;

const SIGN_IN: &str = "solana:signIn";
const SIGN_MESSAGE: &str = "solana:signMessage";
const CONNECT: &str = "standard:connect";

thread_local! {
	/// Every wallet that registered, in order.
	static WALLETS: RefCell<Vec<JsValue>> = const { RefCell::new(Vec::new()) };
	/// Whether this page has announced itself.
	static LISTENING: Cell<bool> = const { Cell::new(false) };
}

/// The Solana wallets the page has, best first: those that sign SIWS
/// messages themselves before those that only sign text.
pub(crate) fn apps() -> Result<Vec<WalletApp>, ErrorResponse> {
	listen()?;
	let mut apps: Vec<WalletApp> = WALLETS.with_borrow(|wallets| {
		wallets
			.iter()
			.filter(|wallet| usable(wallet))
			.map(|wallet| {
				WalletApp {
					name: text(wallet, "name").unwrap_or_default(),
					icon: text(wallet, "icon").unwrap_or_default(),
					sign_in: feature(wallet, SIGN_IN).is_some(),
				}
			})
			.collect()
	});
	apps.sort_by_key(|app| !app.sign_in);
	apps.dedup_by(|later, earlier| later.name == earlier.name);
	Ok(apps)
}

/// Asks the wallet named `name` to sign the message `input` describes.
pub(crate) async fn sign(name: &str, input: &SiwsInput) -> Result<WalletProof, ErrorResponse> {
	listen()?;
	let wallet = WALLETS
		.with_borrow(|wallets| {
			wallets
				.iter()
				.find(|wallet| usable(wallet) && text(wallet, "name").as_deref() == Some(name))
				.cloned()
		})
		.ok_or_else(|| error("no_wallet", "there's no Solana wallet by that name here"))?;

	if let Some(sign_in) = feature(&wallet, SIGN_IN) {
		return sign_in_with(&sign_in, input).await;
	}

	sign_message_with(&wallet, input).await
}

/// `solana:signIn`: the wallet writes and signs the message.
async fn sign_in_with(feature: &JsValue, input: &SiwsInput) -> Result<WalletProof, ErrorResponse> {
	let json = serde_json::to_string(input).map_err(|_| failed())?;
	let input = js_sys::JSON::parse(&json).map_err(|_| failed())?;
	let outputs = call(feature, "signIn", &[input]).await?;
	let output = Array::from(&outputs).get(0);
	let account = get(&output, "account")?;
	let address =
		Pubkey::parse(text(&account, "address").ok_or_else(failed)?).map_err(|_| failed())?;
	Ok(WalletProof {
		address,
		message: String::from_utf8(bytes(&output, "signedMessage")?).map_err(|_| failed())?,
		signature: signature(&output)?,
	})
}

/// `standard:connect`, then `solana:signMessage` over the message's text.
async fn sign_message_with(
	wallet: &JsValue,
	input: &SiwsInput,
) -> Result<WalletProof, ErrorResponse> {
	let connect = feature(wallet, CONNECT).ok_or_else(unsupported)?;
	let sign_message = feature(wallet, SIGN_MESSAGE).ok_or_else(unsupported)?;
	let connected = call(&connect, "connect", &[]).await?;
	let accounts = Array::from(&get(&connected, "accounts")?);
	let account = accounts
		.iter()
		.find(|account| {
			let address = text(account, "address");
			match &input.address {
				Some(wanted) => address.as_deref() == Some(wanted.as_str()),
				None => solana(account),
			}
		})
		.ok_or_else(|| {
			error(
				"wallet_failed",
				"the wallet has no Solana account to sign with",
			)
		})?;
	let address =
		Pubkey::parse(text(&account, "address").ok_or_else(failed)?).map_err(|_| failed())?;
	let message = input.message(&address);
	let request = Object::new();
	set(&request, "account", &account)?;
	set(
		&request,
		"message",
		&Uint8Array::from(message.as_bytes()).into(),
	)?;
	let outputs = call(&sign_message, "signMessage", &[request.into()]).await?;
	let output = Array::from(&outputs).get(0);

	// Some wallets sign a prefixed message; only the exact text counts.
	if bytes(&output, "signedMessage").is_ok_and(|signed| signed != message.as_bytes()) {
		return Err(failed());
	}

	Ok(WalletProof {
		address,
		message,
		signature: signature(&output)?,
	})
}

/// Announces the page to wallets, once, and listens for wallets that load
/// later.
fn listen() -> Result<(), ErrorResponse> {
	if LISTENING.get() {
		return Ok(());
	}

	let window = web_sys::window().ok_or_else(unsupported)?;
	// Wallets call `register(...wallets)`; four covers every wallet seen in
	// practice, which registers one at a time.
	let register = Closure::<dyn Fn(JsValue, JsValue, JsValue, JsValue) -> JsValue>::new(
		|first: JsValue, second: JsValue, third: JsValue, fourth: JsValue| {
			WALLETS.with_borrow_mut(|wallets| {
				for wallet in [first, second, third, fourth] {
					if wallet.is_object() && !wallets.iter().any(|known| Object::is(known, &wallet))
					{
						wallets.push(wallet);
					}
				}
			});
			// What `register` returns: a function that unregisters, which the
			// page never calls.
			Closure::<dyn Fn()>::new(|| {}).into_js_value()
		},
	);
	let api = Object::new();
	set(&api, "register", register.as_ref())?;
	// The page lives as long as these listeners.
	register.forget();

	let api_for_wallets = api.clone();
	let on_register = Closure::<dyn Fn(web_sys::Event)>::new(move |event: web_sys::Event| {
		let detail = event
			.dyn_ref::<web_sys::CustomEvent>()
			.map(web_sys::CustomEvent::detail);

		if let Some(callback) = detail.and_then(|detail| detail.dyn_into::<Function>().ok()) {
			// A wallet's own callback throwing is that wallet's problem.
			let _ = callback.call1(&JsValue::UNDEFINED, &api_for_wallets);
		}
	});
	window
		.add_event_listener_with_callback(
			"wallet-standard:register-wallet",
			on_register.as_ref().unchecked_ref(),
		)
		.map_err(|_| unsupported())?;
	on_register.forget();

	let init = web_sys::CustomEventInit::new();
	init.set_detail(&api);
	let ready = web_sys::CustomEvent::new_with_event_init_dict("wallet-standard:app-ready", &init)
		.map_err(|_| unsupported())?;
	window.dispatch_event(&ready).map_err(|_| unsupported())?;
	LISTENING.set(true);
	Ok(())
}

/// Whether a wallet works with Solana: it names a `solana:` chain and can
/// sign.
fn usable(wallet: &JsValue) -> bool {
	let chains = Reflect::get(wallet, &"chains".into())
		.ok()
		.filter(Array::is_array)
		.map(|chains| Array::from(&chains));
	let on_solana = chains.is_some_and(|chains| {
		chains.iter().any(|chain| {
			chain
				.as_string()
				.is_some_and(|chain| chain.starts_with("solana:"))
		})
	});
	on_solana
		&& (feature(wallet, SIGN_IN).is_some()
			|| (feature(wallet, CONNECT).is_some() && feature(wallet, SIGN_MESSAGE).is_some()))
}

/// Whether an account is on a Solana chain.
fn solana(account: &JsValue) -> bool {
	Reflect::get(account, &"chains".into())
		.ok()
		.filter(Array::is_array)
		.is_some_and(|chains| {
			Array::from(&chains).iter().any(|chain| {
				chain
					.as_string()
					.is_some_and(|chain| chain.starts_with("solana:"))
			})
		})
}

fn feature(wallet: &JsValue, name: &str) -> Option<JsValue> {
	let features = Reflect::get(wallet, &"features".into()).ok()?;
	Reflect::get(&features, &name.into())
		.ok()
		.filter(JsValue::is_object)
}

/// Calls `object.method(...arguments)` and waits for its promise.
async fn call(
	object: &JsValue,
	method: &str,
	arguments: &[JsValue],
) -> Result<JsValue, ErrorResponse> {
	let function: Function = Reflect::get(object, &method.into())
		.ok()
		.and_then(|function| function.dyn_into().ok())
		.ok_or_else(unsupported)?;
	let arguments: Array = arguments.iter().collect();
	let promise: Promise = function
		.apply(object, &arguments)
		.map_err(|error| refused(&error))?
		.dyn_into()
		.map_err(|_| failed())?;
	JsFuture::from(promise)
		.await
		.map_err(|error| refused(&error))
}

fn get(object: &JsValue, key: &str) -> Result<JsValue, ErrorResponse> {
	Reflect::get(object, &key.into())
		.ok()
		.filter(|value| !value.is_undefined() && !value.is_null())
		.ok_or_else(failed)
}

fn set(object: &Object, key: &str, value: &JsValue) -> Result<(), ErrorResponse> {
	Reflect::set(object, &key.into(), value)
		.map(drop)
		.map_err(|_| failed())
}

fn text(object: &JsValue, key: &str) -> Option<String> {
	Reflect::get(object, &key.into()).ok()?.as_string()
}

fn bytes(object: &JsValue, key: &str) -> Result<Vec<u8>, ErrorResponse> {
	let value = get(object, key)?;
	value
		.dyn_into::<Uint8Array>()
		.map(|array| array.to_vec())
		.map_err(|_| failed())
}

fn signature(output: &JsValue) -> Result<Signature, ErrorResponse> {
	let bytes: [u8; 64] = bytes(output, "signature")?
		.try_into()
		.map_err(|_| failed())?;
	Ok(Signature::from_bytes(bytes))
}

/// What a wallet's refusal means: the person said no, or it failed.
fn refused(thrown: &JsValue) -> ErrorResponse {
	let code = Reflect::get(thrown, &"code".into())
		.ok()
		.and_then(|code| code.as_f64());
	let words = [
		text(thrown, "name").unwrap_or_default(),
		text(thrown, "message").unwrap_or_default(),
	]
	.join(" ")
	.to_lowercase();

	// 4001 is the provider-wide "user rejected the request".
	if code == Some(4001.0) || words.contains("reject") || words.contains("cancel") {
		error("cancelled", "the wallet request was cancelled")
	} else {
		failed()
	}
}

fn failed() -> ErrorResponse {
	error(
		"wallet_failed",
		"the wallet's answer didn't work; try again",
	)
}

fn unsupported() -> ErrorResponse {
	error("unsupported", "this browser can't reach Solana wallets")
}
