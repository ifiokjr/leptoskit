//! Signing in with an emailed code. The same form signs up: a new address
//! gets an account.

use leptoskit::prelude::*;

use crate::store::Store;
use crate::ui;

/// The sign-in form, shown in place of every screen until someone signs in.
#[component]
pub fn Form(store: &Store) -> Element {
	let form = store.with(|state| state.sign_in.clone());
	let card = view! { <ui::PanelCard /> }
		.child(view! { <ui::Title text="Sign in to Commonplace" /> })
		.child(match &form.error {
			Some(error) => view! { <ui::Problem message=error /> },
			None => view! { <View /> },
		});

	if form.sent {
		card.child(view! {
			<Text>
				{format!(
					"We sent a code to {}. It works for 10 minutes.",
					form.email.trim()
				)}
			</Text>
		})
		.child(view! {
			<ui::Field
				label="Code"
				value=&form.code
				placeholder="K7QM-R2XD"
				on_change=|value| {
					Store::current().update(|state| state.sign_in.code = value.to_owned());
				}
			/>
		})
		.child(view! {
			<ui::Primary
				label="Sign in"
				enabled=!form.busy && !form.code.trim().is_empty()
				on_press=|| {
					Store::current().verify_code();
				}
			/>
		})
		.child(view! {
			<ui::Secondary
				label="Use another address"
				on_press=|| {
					Store::current().update(|state| {
						state.sign_in.sent = false;
						state.sign_in.code.clear();
					});
				}
			/>
		})
	} else {
		card.child(view! {
			<Text>
				"Enter your email address, and we'll send you a code. New here? The same code creates \
			 your account."
			</Text>
		})
		.child(view! {
			<ui::Field
				label="Email"
				value=&form.email
				placeholder="you@example.com"
				on_change=|value| {
					Store::current().update(|state| state.sign_in.email = value.to_owned());
				}
			/>
		})
		.child(view! {
			<ui::Primary
				label="Email me a code"
				enabled=!form.busy
				on_press=|| Store::current().send_code()
			/>
		})
	}
}
