//! Signing in with an emailed code. The same form signs up: a new address
//! gets an account.

use leptoskit::prelude::*;

use crate::store::Store;
use crate::ui;

/// The sign-in form, shown in place of every screen until someone signs in.
pub fn form(store: &Store) -> Element {
	let form = store.with(|state| state.sign_in.clone());
	let card = ui::card()
		.child(ui::title("Sign in to Commonplace"))
		.child(match &form.error {
			Some(error) => ui::problem(error),
			None => View(),
		});

	if form.sent {
		card.child(Text(format!(
			"We sent a code to {}. It works for 10 minutes.",
			form.email.trim()
		)))
		.child(ui::field("Code", &form.code, "K7QM-R2XD", |value| {
			Store::current().update(|state| state.sign_in.code = value.to_owned());
		}))
		.child(ui::primary(
			"Sign in",
			!form.busy && !form.code.trim().is_empty(),
			|| {
				Store::current().verify_code();
			},
		))
		.child(ui::secondary("Use another address", || {
			Store::current().update(|state| {
				state.sign_in.sent = false;
				state.sign_in.code.clear();
			});
		}))
	} else {
		card.child(Text(
			"Enter your email address, and we'll send you a code. New here? The same code creates \
			 your account.",
		))
		.child(ui::field(
			"Email",
			&form.email,
			"you@example.com",
			|value| {
				Store::current().update(|state| state.sign_in.email = value.to_owned());
			},
		))
		.child(ui::primary("Email me a code", !form.busy, || {
			Store::current().send_code()
		}))
	}
}
