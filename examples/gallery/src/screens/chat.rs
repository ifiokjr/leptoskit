//! A chat thread: grouped bubbles that wrap to three quarters of the width,
//! a photo message, a typing indicator, and a composer.

use leptoskit::prelude::Button;
use leptoskit::prelude::Image;
use leptoskit::prelude::ScrollView;
use leptoskit::prelude::Text;
use leptoskit::prelude::TextInput;
use leptoskit::prelude::View;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_yoga::Align;

use crate::ScreenState;
use crate::ui::circle;
use crate::ui::hex;
use crate::ui::label;
use crate::ui::spacer;
use crate::ui::type_scale;

struct Palette {
	surface: Color,
	ink: Color,
	muted: Color,
	line: Color,
	accent: Color,
	incoming: Color,
	online: Color,
}

fn palette() -> Palette {
	Palette {
		surface: hex("#ffffff"),
		ink: hex("#111827"),
		muted: hex("#8a8f98"),
		line: hex("#e7e7ea"),
		accent: hex("#4f46e5"),
		incoming: hex("#f0f0f3"),
		online: hex("#16a34a"),
	}
}

/// Who sent a message.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sender {
	Them,
	Me,
}

/// One entry in the thread.
enum Message {
	Text(Sender, &'static str),
	Photo(Sender, usize),
}

impl Message {
	fn sender(&self) -> Sender {
		match self {
			Self::Text(sender, _) | Self::Photo(sender, _) => *sender,
		}
	}
}

const THREAD: [Message; 7] = [
	Message::Text(
		Sender::Them,
		"Are you still coming to the type walk on Saturday?",
	),
	Message::Text(
		Sender::Them,
		"Kwame says the Accra foundry is opening its archive room for us 👀",
	),
	Message::Text(Sender::Me, "Yes! Wouldn't miss it."),
	Message::Text(
		Sender::Me,
		"I'm bringing the specimen book so we can compare the old wood type against the scans.",
	),
	Message::Photo(Sender::Me, 3),
	Message::Text(
		Sender::Them,
		"That cover is beautiful. Meet at the gate at 10?",
	),
	Message::Text(Sender::Me, "10 works. I'll save you a coffee ☕️"),
];

fn align_for(sender: Sender) -> Align {
	match sender {
		Sender::Them => Align::FlexStart,
		Sender::Me => Align::FlexEnd,
	}
}

fn bubble(message: &Message, state: &ScreenState, colors: &Palette) -> Element {
	let sender = message.sender();
	match message {
		Message::Text(_, text) => {
			let (background, foreground) = match sender {
				Sender::Them => (colors.incoming, colors.ink),
				Sender::Me => (colors.accent, colors.surface),
			};
			View()
				.with_style(|style| style.align_self = align_for(sender))
				.class("max-w-[75%] py-2 px-3")
				.background(background)
				.class("rounded-[18px]")
				.child(label(*text, type_scale::BODY, foreground))
		}
		Message::Photo(_, photo) => {
			Image(state.assets.photo(*photo))
				.with_style(|style| style.align_self = align_for(sender))
				.class("w-[180px] h-[180px] rounded-[18px]")
		}
	}
}

/// Three dots in an incoming bubble.
fn typing(colors: &Palette) -> Element {
	View()
		.class("flex-row items-center self-start gap-1 p-3.5")
		.background(colors.incoming)
		.class("rounded-[18px]")
		.child(circle(8.0, colors.muted).class("opacity-[0.9]"))
		.child(circle(8.0, colors.muted).class("opacity-[0.6]"))
		.child(circle(8.0, colors.muted).class("opacity-[0.35]"))
}

fn thread(state: &ScreenState, colors: &Palette) -> Element {
	let mut column = View().class("flex-col py-3 px-4 gap-0.5").child(
		View()
			.class("pb-2")
			.child(label("Today 9:41", type_scale::CAPTION, colors.muted).class("text-center")),
	);
	let mut previous: Option<Sender> = None;
	for message in &THREAD {
		let mut bubble = bubble(message, state, colors);
		// A change of sender opens a new group with more space above it.
		if previous.is_some_and(|sender| sender != message.sender()) {
			bubble = bubble.class("mt-2");
		}
		previous = Some(message.sender());
		column = column.child(bubble);
	}
	column
		.child(label("Read 9:44", type_scale::CAPTION, colors.muted).class("self-end mt-1"))
		.child(typing(colors).class("mt-2"))
}

fn header(state: &ScreenState, colors: &Palette) -> Element {
	let action = |icon: Element| {
		View()
			.class("w-9 h-9 rounded-[18px]")
			.background(colors.incoming)
			.class("items-center justify-center")
			.child(icon)
	};
	View()
		.class("flex-col")
		.child(
			View()
				.class("flex-row items-center gap-3 pt-[52px] pr-4 pb-3 pl-3")
				.child(
					Text("‹")
						.class("text-[34px] font-normal")
						.color(colors.accent),
				)
				.child(Image(state.assets.avatar(1)).class("w-10 h-10 rounded-[20px]"))
				.child(
					View()
						.class("flex-col gap-0.5")
						.child(label("Kemi Adeyemi", type_scale::HEADLINE, colors.ink))
						.child(
							View()
								.class("flex-row items-center gap-1.5")
								.child(circle(7.0, colors.online))
								.child(label("Active now", type_scale::CAPTION, colors.muted)),
						),
				)
				.child(spacer())
				.child(action(
					View()
						.class("w-[17px] h-3 rounded-[3px]")
						.background(colors.accent),
				))
				.child(action(
					View()
						.class("w-3.5 h-3.5 rounded-[7px]")
						.border(2.5, colors.accent),
				)),
		)
		.child(View().background(colors.line).class("h-[1px]"))
}

fn composer(colors: &Palette) -> Element {
	View()
		.class("flex-col")
		.child(View().background(colors.line).class("h-[1px]"))
		.child(
			View()
				.class("flex-row items-center gap-2 pt-2 px-3 pb-[30px]")
				.child(
					Button("+")
						.accessibility_label("Add an attachment")
						.class("text-[22px] font-normal")
						.color(colors.muted)
						.background(colors.incoming)
						.class("rounded-[18px] w-9 h-9"),
				)
				.child(
					View()
						.class("flex-row items-center flex-1 h-9 py-0 px-3.5")
						.border(1.0, colors.line)
						.class("rounded-[18px]")
						.child(
							TextInput()
								.placeholder("Message")
								.accessibility_label("Message")
								.font(type_scale::BODY.size, type_scale::BODY.weight)
								.color(colors.ink)
								.class("bg-[#00000000]")
								.class("h-5 flex-1"),
						),
				)
				.child(
					Button("↑")
						.accessibility_label("Send message")
						.class("text-[18px] font-bold")
						.color(colors.surface)
						.background(colors.accent)
						.class("rounded-[18px] w-9 h-9"),
				),
		)
}

/// The chat screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = palette();
	View()
		.class("flex-col w-full h-full")
		.background(colors.surface)
		.child(header(state, &colors))
		.child(ScrollView().class("flex-1").child(thread(state, &colors)))
		.child(composer(&colors))
}
