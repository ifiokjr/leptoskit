//! "Summarize this channel": an action, because a summarizer is usually an
//! outside service that's slow, costs money, and mustn't run twice inside a
//! retried transaction.
//!
//! The action reads the channel as the caller, through the [`Messages`]
//! query, asks a [`Summarizer`], and pins the result with a mutation, so
//! policies and receipts still apply. Huddle ships [`LocalSummarizer`], which
//! is deterministic and calls nothing; a real app swaps in a model behind
//! the same trait.
//!
//! [`Messages`]: crate::Messages

use std::fmt::Write as _;

use huddle_db::Channel;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;

use crate::MessageView;

/// Turns a channel's messages into a few sentences.
pub trait Summarizer: Send + Sync + 'static {
	/// A summary of `messages`, oldest first, from the channel `name`.
	fn summarize(&self, name: &str, messages: &[MessageView]) -> String;
}

/// A summarizer that counts instead of calling a model: who spoke most, the
/// words that came up most, and the latest message. The same messages
/// always give the same summary.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalSummarizer;

/// Words too common to call a topic.
const COMMON: &[&str] = &[
	"about", "after", "again", "also", "been", "before", "being", "could", "does", "doing", "from",
	"have", "here", "just", "like", "more", "much", "need", "only", "really", "should", "some",
	"than", "that", "their", "them", "then", "there", "these", "they", "this", "today", "tomorrow",
	"very", "want", "were", "what", "when", "where", "which", "while", "will", "with", "would",
	"your",
];

impl Summarizer for LocalSummarizer {
	fn summarize(&self, name: &str, messages: &[MessageView]) -> String {
		let Some(latest) = messages.last() else {
			return format!("Nothing has been said in #{name} yet.");
		};

		let mut speakers: Vec<(String, usize)> = Vec::new();
		let mut words: Vec<(String, usize)> = Vec::new();

		for message in messages {
			bump(&mut speakers, &message.author);

			for word in message
				.body
				.split(|character: char| !character.is_alphanumeric())
				.map(str::to_lowercase)
				.filter(|word| word.chars().count() > 3 && !COMMON.contains(&word.as_str()))
			{
				bump(&mut words, &word);
			}
		}

		// Most first; ties alphabetically, so the summary never wobbles.
		let order = |a: &(String, usize), b: &(String, usize)| b.1.cmp(&a.1).then(a.0.cmp(&b.0));
		speakers.sort_by(order);
		words.sort_by(order);

		let people = match speakers.len() {
			1 => "1 person".to_owned(),
			count => format!("{count} people"),
		};
		let said = match messages.len() {
			1 => "1 message".to_owned(),
			count => format!("{count} messages"),
		};
		let busiest = &speakers[0];
		let mut summary = format!(
			"{said} from {people} in #{name}. {} said the most ({}).",
			busiest.0, busiest.1
		);

		if !words.is_empty() {
			let topics: Vec<&str> = words
				.iter()
				.take(3)
				.map(|(word, _)| word.as_str())
				.collect();
			let _ = write!(summary, " Came up most: {}.", topics.join(", "));
		}

		let excerpt: String = latest.body.chars().take(80).collect();
		let ellipsis = if latest.body.chars().count() > 80 {
			"…"
		} else {
			""
		};
		let _ = write!(
			summary,
			" Latest, from {}: “{excerpt}{ellipsis}”",
			latest.author
		);
		summary
	}
}

fn bump(counts: &mut Vec<(String, usize)>, key: &str) {
	match counts.iter_mut().find(|(seen, _)| seen == key) {
		Some((_, count)) => *count += 1,
		None => counts.push((key.to_owned(), 1)),
	}
}

/// Summarizes the channel's latest 200 messages, pins the summary for
/// everyone in it, and returns it.
#[rpc::action]
pub async fn summarize_channel(cx: &rpc::ActionCx, channel: Id<Channel>) -> rpc::Result<String> {
	let cards = cx.query(crate::Channels {}).await?;
	let name = cards
		.iter()
		.find(|card| card.id == channel)
		.map(|card| card.name.clone())
		.ok_or_else(|| rpc::Error::not_found("no such channel"))?;
	let messages = cx
		.query(crate::Messages {
			channel,
			limit: 200,
			before: None,
		})
		.await?;
	let summary = crate::services::get()
		.summarizer
		.summarize(&name, &messages);
	cx.mutate(PinSummary {
		channel,
		summary: summary.clone(),
	})
	.await?;
	Ok(summary)
}

/// Pins `summary` above a channel's messages, for everyone in it.
#[rpc::mutation]
pub async fn pin_summary(
	cx: &rpc::MutationCx,
	channel: Id<Channel>,
	summary: String,
) -> rpc::Result<()> {
	let summary: String = summary.chars().take(2_000).collect();
	Channel::update()
		.filter(|row| row.id.eq(channel))
		.set(|row| row.summary, Some(summary))
		.run(cx.tx())
		.await?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn said(author: &str, body: &str) -> MessageView {
		MessageView {
			id: Id::parse("0192f3c4-0000-7000-8000-000000000001").expect("an ID"),
			author_id: Id::parse("0192f3c4-0000-7000-8000-000000000002").expect("an ID"),
			author: author.to_owned(),
			body: body.to_owned(),
			client_key: "k".to_owned(),
			sent_at: String::new(),
		}
	}

	#[test]
	fn the_same_messages_give_the_same_summary() {
		let messages = [
			said("Ada", "The launch checklist is ready"),
			said("Grace", "Launch on Thursday, after the checklist review"),
			said("Ada", "Checklist approved"),
		];
		let summary = LocalSummarizer.summarize("launch", &messages);
		assert_eq!(
			summary,
			"3 messages from 2 people in #launch. Ada said the most (2). Came up most: checklist, \
			 launch, approved. Latest, from Ada: “Checklist approved”"
		);
		assert_eq!(summary, LocalSummarizer.summarize("launch", &messages));
	}

	#[test]
	fn an_empty_channel_says_so() {
		assert_eq!(
			LocalSummarizer.summarize("general", &[]),
			"Nothing has been said in #general yet."
		);
	}
}
