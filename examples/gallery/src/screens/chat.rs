//! A chat thread: grouped bubbles that wrap to three quarters of the width,
//! a photo message, a typing indicator, and a composer.

use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_native::TextAlign;
use leptoskit_yoga::Align;
use leptoskit_yoga::JustifyContent;

use crate::ScreenState;
use crate::ui::Layout;
use crate::ui::circle;
use crate::ui::hex;
use crate::ui::label;
use crate::ui::space;
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

const BUBBLE_RADIUS: f32 = 18.0;

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
			Element::view()
				.align_self(align_for(sender))
				.max_width_fraction(0.75)
				.padding(space::S, space::M)
				.background(background)
				.corner_radius(BUBBLE_RADIUS)
				.child(label(*text, type_scale::BODY, foreground))
		}
		Message::Photo(_, photo) => {
			Element::image(state.assets.photo(*photo))
				.align_self(align_for(sender))
				.size(180.0, 180.0)
				.corner_radius(BUBBLE_RADIUS)
		}
	}
}

/// Three dots in an incoming bubble.
fn typing(colors: &Palette) -> Element {
	Element::view()
		.row()
		.align_self(Align::FlexStart)
		.gap(space::XS)
		.padding(space::M + 2.0, space::M + 2.0)
		.background(colors.incoming)
		.corner_radius(BUBBLE_RADIUS)
		.child(circle(8.0, colors.muted).opacity(0.9))
		.child(circle(8.0, colors.muted).opacity(0.6))
		.child(circle(8.0, colors.muted).opacity(0.35))
}

fn thread(state: &ScreenState, colors: &Palette) -> Element {
	let mut column = Element::view()
		.column()
		.padding(space::M, space::L)
		.gap(space::XXS)
		.child(
			Element::view()
				.padding_edges(0.0, 0.0, space::S, 0.0)
				.child(
					label("Today 9:41", type_scale::CAPTION, colors.muted)
						.text_align(TextAlign::Center),
				),
		);
	let mut previous: Option<Sender> = None;
	for message in &THREAD {
		let mut bubble = bubble(message, state, colors);
		// A change of sender opens a new group with more space above it.
		if previous.is_some_and(|sender| sender != message.sender()) {
			bubble = bubble.margin_top(space::S);
		}
		previous = Some(message.sender());
		column = column.child(bubble);
	}
	column
		.child(
			label("Read 9:44", type_scale::CAPTION, colors.muted)
				.align_self(Align::FlexEnd)
				.margin_top(space::XS),
		)
		.child(typing(colors).margin_top(space::S))
}

fn header(state: &ScreenState, colors: &Palette) -> Element {
	let action = |icon: Element| {
		Element::view()
			.size(36.0, 36.0)
			.corner_radius(18.0)
			.background(colors.incoming)
			.align(Align::Center)
			.justify(JustifyContent::Center)
			.child(icon)
	};
	Element::view()
		.column()
		.child(
			Element::view()
				.row()
				.gap(space::M)
				.padding_edges(52.0, space::L, space::M, space::M)
				.child(
					Element::text("‹")
						.font(34.0, FontWeight::Regular)
						.color(colors.accent),
				)
				.child(
					Element::image(state.assets.avatar(1))
						.size(40.0, 40.0)
						.corner_radius(20.0),
				)
				.child(
					Element::view()
						.column()
						.gap(space::XXS)
						.child(label("Kemi Adeyemi", type_scale::HEADLINE, colors.ink))
						.child(
							Element::view()
								.row()
								.gap(space::XS + 2.0)
								.child(circle(7.0, colors.online))
								.child(label("Active now", type_scale::CAPTION, colors.muted)),
						),
				)
				.child(spacer())
				.child(action(
					Element::view()
						.size(17.0, 12.0)
						.corner_radius(3.0)
						.background(colors.accent),
				))
				.child(action(
					Element::view()
						.size(14.0, 14.0)
						.corner_radius(7.0)
						.border(2.5, colors.accent),
				)),
		)
		.child(Element::view().background(colors.line).height(1.0))
}

fn composer(colors: &Palette) -> Element {
	Element::view()
		.column()
		.child(Element::view().background(colors.line).height(1.0))
		.child(
			Element::view()
				.row()
				.gap(space::S)
				.padding_edges(space::S, space::M, 30.0, space::M)
				.child(
					Element::button("+")
						.font(22.0, FontWeight::Regular)
						.color(colors.muted)
						.background(colors.incoming)
						.corner_radius(18.0)
						.size(36.0, 36.0),
				)
				.child(
					Element::view()
						.row()
						.grow()
						.height(36.0)
						.padding(0.0, space::M + 2.0)
						.border(1.0, colors.line)
						.corner_radius(18.0)
						.child(
							Element::text_input()
								.with_props(|props| props.placeholder = Some("Message".to_owned()))
								.font(type_scale::BODY.size, type_scale::BODY.weight)
								.color(colors.ink)
								.background(hex("#00000000"))
								.height(20.0)
								.grow(),
						),
				)
				.child(
					Element::button("↑")
						.font(18.0, FontWeight::Bold)
						.color(colors.surface)
						.background(colors.accent)
						.corner_radius(18.0)
						.size(36.0, 36.0),
				),
		)
}

/// The chat screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = palette();
	Element::view()
		.column()
		.fill()
		.background(colors.surface)
		.child(header(state, &colors))
		.child(Element::scroll_view().grow().child(thread(state, &colors)))
		.child(composer(&colors))
}
