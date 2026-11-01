//! A task board per channel.
//!
//! Moving a card renumbers the column it lands in, inside one mutation.
//! Mutations run serializably, so when two people move cards at the same
//! moment one of them runs again on top of the other's result, and both
//! screens end on the same order.

pub use huddle_db::COLUMNS;
use huddle_db::Channel;
use huddle_db::Task;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// A card on the board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskCard {
	pub id: Id<Task>,
	pub title: String,
	/// `todo`, `doing`, or `done`.
	pub column: String,
	/// Its place in the column, from 0.
	pub position: i64,
}

/// A channel's cards, column by column, each column in order.
#[rpc::query]
pub async fn board(cx: &rpc::QueryCx, channel: Id<Channel>) -> rpc::Result<Vec<TaskCard>> {
	let tasks = Task::query()
		.filter(|task| task.channel_id.eq(channel))
		.order_by(|task| task.position.asc())
		.all(cx.db())
		.await?;
	let mut cards: Vec<TaskCard> = tasks
		.into_iter()
		.map(|task| {
			TaskCard {
				id: task.id,
				title: task.title,
				column: task.column,
				position: task.position,
			}
		})
		.collect();
	cards.sort_by_key(|card| {
		(
			COLUMNS.iter().position(|column| *column == card.column),
			card.position,
		)
	});
	Ok(cards)
}

/// Adds a card at the bottom of `todo`.
#[rpc::mutation]
pub async fn add_task(
	cx: &rpc::MutationCx,
	channel: Id<Channel>,
	title: String,
) -> rpc::Result<Id<Task>> {
	let title = title.trim().to_owned();

	if title.is_empty() {
		return Err(rpc::Error::invalid("a card needs a title"));
	}

	let position = Task::query()
		.filter(|task| task.channel_id.eq(channel) & task.column.eq("todo"))
		.count(cx.tx())
		.await?;
	let task = huddle_db::NewTask {
		channel_id: channel,
		title,
		column: "todo".to_owned(),
		position: i64::try_from(position).unwrap_or(i64::MAX),
	}
	.insert(cx.tx())
	.await?;
	Ok(task.id)
}

/// Moves a card to `column`, at `index` among the cards already there (past
/// the end puts it last), and renumbers both columns it touched.
#[rpc::mutation]
pub async fn move_task(
	cx: &rpc::MutationCx,
	task: Id<Task>,
	column: String,
	index: u32,
) -> rpc::Result<()> {
	if !COLUMNS.contains(&column.as_str()) {
		return Err(rpc::Error::invalid(format!("there's no `{column}` column")));
	}

	let moving = Task::get(cx.tx(), task).await?;
	let channel = moving.channel_id;
	let from = moving.column.clone();
	let target = column.clone();
	let mut others = Task::query()
		.filter(|row| row.channel_id.eq(channel) & row.column.eq(target.clone()) & row.id.ne(task))
		.order_by(|row| row.position.asc())
		.all(cx.tx())
		.await?;
	let at = (index as usize).min(others.len());
	let mut moved = moving;
	moved.column = column;
	others.insert(at, moved);
	renumber(cx, others).await?;

	if from != target {
		let left = Task::query()
			.filter(|row| row.channel_id.eq(channel) & row.column.eq(from.clone()))
			.order_by(|row| row.position.asc())
			.all(cx.tx())
			.await?;
		renumber(cx, left).await?;
	}

	Ok(())
}

/// Saves each card whose column or place changed.
#[cfg(feature = "server")]
async fn renumber(cx: &rpc::MutationCx, cards: Vec<Task>) -> rpc::Result<()> {
	for (position, mut card) in cards.into_iter().enumerate() {
		let position = i64::try_from(position).unwrap_or(i64::MAX);
		let column = card.column.clone();
		let original = Task::get(cx.tx(), card.id).await?;

		if original.position != position || original.column != column {
			card.position = position;
			card.save(cx.tx()).await?;
		}
	}

	Ok(())
}
