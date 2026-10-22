//! A chat thread: grouped bubbles that wrap to three quarters of the width,
//! a photo message, a typing indicator, and a composer.

use leptoskit::prelude::*;
use leptoskit_native::Color;
use leptoskit_native::Role;
use leptoskit_yoga::Align;

use crate::ScreenState;
use crate::ui::Circle;
use crate::ui::Label;
use crate::ui::hex;
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

/// One message, aligned to its sender's side.
#[component]
fn Bubble(message: &Message, state: &ScreenState, colors: &Palette) -> Element {
	let sender = message.sender();
	match message {
		Message::Text(_, text) => {
			let (background, foreground) = match sender {
				Sender::Them => (colors.incoming, colors.ink),
				Sender::Me => (colors.accent, colors.surface),
			};
			view! {
				<View
					with_style=|style| style.align_self = align_for(sender)
					class="py-2 px-3 max-w-[75%]"
					background=background
					class="rounded-[18px]"
				>
					<Label style=type_scale::BODY color=foreground>
						{*text}
					</Label>
				</View>
			}
		}
		Message::Photo(_, photo) => {
			view! {
				<Image
					source=state.assets.photo(*photo)
					with_style=|style| style.align_self = align_for(sender)
					class="w-[180px] h-[180px] rounded-[18px]"
				/>
			}
		}
	}
}

/// Three dots in an incoming bubble, announced as a polite live region.
#[component]
fn Typing(colors: &Palette) -> Element {
	view! {
		<View
			class="flex-row gap-1 items-center self-start p-3.5"
			background=colors.incoming
			class="rounded-[18px]"
			role=Role::Status
			aria_label="Typing"
		>
			<Circle diameter=8.0 color=colors.muted class="opacity-[0.9]" />
			<Circle diameter=8.0 color=colors.muted class="opacity-[0.6]" />
			<Circle diameter=8.0 color=colors.muted class="opacity-[0.35]" />
		</View>
	}
}

#[component]
fn Thread(state: &ScreenState, colors: &Palette) -> Element {
	let mut previous: Option<Sender> = None;
	let bubbles = THREAD
		.iter()
		.map(|message| {
			let bubble = Bubble(message, state, colors);
			// A change of sender opens a new group with more space above it.
			let opens_group = previous.is_some_and(|sender| sender != message.sender());
			previous = Some(message.sender());
			if opens_group {
				bubble.class("mt-2")
			} else {
				bubble
			}
		})
		.collect::<Fragment>();

	view! {
		<View class="flex-col gap-0.5 py-3 px-4">
			<View class="pb-2">
				<Label style=type_scale::CAPTION color=colors.muted class="text-center">
					"Today 9:41"
				</Label>
			</View>
			{bubbles}
			<Label style=type_scale::CAPTION color=colors.muted class="self-end mt-1">
				"Read 9:44"
			</Label>
			{Typing(colors).class("mt-2")}
		</View>
	}
}

/// A round button in the header.
#[component]
fn HeaderAction(colors: &Palette, children: Children) -> Element {
	view! {
		<View
			class="w-9 h-9 rounded-[18px]"
			background=colors.incoming
			class="justify-center items-center"
		>
			{children}
		</View>
	}
}

#[component]
fn Header(state: &ScreenState, colors: &Palette) -> Element {
	view! {
		<View class="flex-col">
			<View class="flex-row gap-3 items-center pr-4 pb-3 pl-3 pt-safe-offset-3">
				<Text class="font-normal text-[34px]" color=colors.accent>
					"‹"
				</Text>
				<Image source=state.assets.avatar(1) class="w-10 h-10 rounded-[20px]" />
				<View class="flex-col gap-0.5">
					<Label style=type_scale::HEADLINE color=colors.ink>
						"Kemi Adeyemi"
					</Label>
					<View class="flex-row gap-1.5 items-center">
						<Circle diameter=7.0 color=colors.online />
						<Label style=type_scale::CAPTION color=colors.muted>
							"Active now"
						</Label>
					</View>
				</View>
				<View class="flex-1" />
				<HeaderAction colors=colors>
					<View class="h-3 w-[17px] rounded-[3px]" background=colors.accent />
				</HeaderAction>
				<HeaderAction colors=colors>
					<View class="w-3.5 h-3.5 rounded-[7px]" border=(2.5, colors.accent) />
				</HeaderAction>
			</View>
			<View background=colors.line class="h-[1px]" />
		</View>
	}
}

#[component]
fn Composer(colors: &Palette) -> Element {
	view! {
		<View class="flex-col">
			<View background=colors.line class="h-[1px]" />
			<View class="flex-row gap-2 items-center px-3 pt-2 pb-safe-offset-2">
				<Button
					accessibility_label="Add an attachment"
					class="font-normal text-[22px]"
					color=colors.muted
					background=colors.incoming
					class="w-9 h-9 rounded-[18px]"
				>
					"+"
				</Button>
				<View
					class="flex-row flex-1 items-center py-0 px-3.5 h-9"
					border=(1.0, colors.line)
					class="rounded-[18px]"
				>
					<TextInput
						placeholder="Message"
						accessibility_label="Message"
						font=(type_scale::BODY.size, type_scale::BODY.weight)
						color=colors.ink
						class="bg-[#00000000]"
						class="flex-1 h-5"
					/>
				</View>
				<Button
					accessibility_label="Send message"
					class="font-bold text-[18px]"
					color=colors.surface
					background=colors.accent
					class="w-9 h-9 rounded-[18px]"
				>
					"↑"
				</Button>
			</View>
		</View>
	}
}

/// The chat screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = &palette();
	view! {
		<View class="flex-col w-full h-full" background=colors.surface>
			<Header state=state colors=colors />
			<ScrollView class="flex-1">
				<Thread state=state colors=colors />
			</ScrollView>
			<Composer colors=colors />
		</View>
	}
}
