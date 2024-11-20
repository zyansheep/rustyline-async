use std::collections::VecDeque;

use futures_channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures_util::StreamExt;

pub struct History {
	pub entries: VecDeque<String>,
	pub max_size: usize,
	pub sender: UnboundedSender<String>,
	receiver: UnboundedReceiver<String>,

	current_position: Option<usize>,
}
impl Default for History {
	fn default() -> Self {
		let (sender, receiver) = mpsc::unbounded();
		Self {
			entries: Default::default(),
			max_size: 1000,
			sender,
			receiver,
			current_position: Default::default(),
		}
	}
}

impl History {
	// Update history entries
	pub async fn update(&mut self) {
		// Receive a new line
		if let Some(line) = self.receiver.next().await {
			// Reset offset to newest entry
			self.current_position = None;
			// Don't add entry if last entry was same, or line was empty.
			if self.entries.front() == Some(&line) || line.is_empty() {
				return;
			}
			// Add entry to front of history
			self.entries.push_front(line);
			// Check if already have enough entries
			if self.entries.len() > self.max_size {
				// Remove oldest entry
				self.entries.pop_back();
			}
		}
	}

	// Sets the history position back to the start.
	pub fn reset_position(&mut self) {
		self.current_position = None;
	}

	// Find next history that matches a given string from an index
	pub fn search_next(&mut self, _current: &str) -> Option<&str> {
		if let Some(index) = &mut self.current_position {
			if *index < self.entries.len() - 1 {
				*index += 1;
			}
			Some(&self.entries[*index])
		} else if !self.entries.is_empty() {
			self.current_position = Some(0);
			Some(&self.entries[0])
		} else {
			None
		}
	}
	// Find previous history item that matches a given string from an index
	pub fn search_previous(&mut self, _current: &str) -> Option<&str> {
		if let Some(index) = &mut self.current_position {
			if *index == 0 {
				self.current_position = None;
				return Some("");
			}
			*index -= 1;
			Some(&self.entries[*index])
		} else {
			None
		}
	}
}

#[cfg(test)]
#[tokio::test]
async fn test_history() {
	let mut history = History::default();

	history.sender.unbounded_send("foo".into()).unwrap();
	history.update().await;
	history.sender.unbounded_send("bar".into()).unwrap();
	history.update().await;
	history.sender.unbounded_send("baz".into()).unwrap();
	history.update().await;

	for _ in 0..2 {
		// Previous will navigate nowhere.
		assert_eq!(None, history.search_previous(""));

		// Going back in history.
		assert_eq!(Some("baz"), history.search_next(""));
		assert_eq!(Some("bar"), history.search_next(""));
		assert_eq!(Some("foo"), history.search_next(""));

		// Last entry should just repeat.
		assert_eq!(Some("foo"), history.search_next(""));

		// Going forward.
		assert_eq!(Some("bar"), history.search_previous(""));
		assert_eq!(Some("baz"), history.search_previous(""));

		// Alternate.
		assert_eq!(Some("bar"), history.search_next(""));
		assert_eq!(Some("baz"), history.search_previous(""));

		// Back to the beginning. Should return "" once.
		assert_eq!(Some(""), history.search_previous(""));
		assert_eq!(None, history.search_previous(""));

		// Going back again.
		assert_eq!(Some("baz"), history.search_next(""));
		assert_eq!(Some("bar"), history.search_next(""));

		// Resetting the position.
		history.reset_position();
	}
}

#[cfg(test)]
#[tokio::test]
async fn test_history_limit() {
	let mut history = History {
		max_size: 3,
		..Default::default()
	};

	history.sender.unbounded_send("foo".into()).unwrap();
	history.update().await;
	history.sender.unbounded_send("bar".into()).unwrap();
	history.update().await;
	history.sender.unbounded_send("baz".into()).unwrap();
	history.update().await;
	history.sender.unbounded_send("qux".into()).unwrap(); // Should remove "foo".
	history.update().await;

	assert_eq!(Some("qux"), history.search_next(""));
	assert_eq!(Some("baz"), history.search_next(""));
	assert_eq!(Some("bar"), history.search_next(""));
	assert_eq!(Some("bar"), history.search_next(""));
}
