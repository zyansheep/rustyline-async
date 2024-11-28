use std::collections::VecDeque;

pub struct History {
	// Note: old entries in front, new ones at the back.
	entries: VecDeque<String>,
	max_size: usize,
	current_position: Option<usize>,
}
impl Default for History {
	fn default() -> Self {
		Self {
			entries: Default::default(),
			max_size: 1000,
			current_position: Default::default(),
		}
	}
}

impl History {
	// Update history entries
	pub fn add_entry(&mut self, line: String) {
		// Reset offset to newest entry
		self.current_position = None;
		// Don't add entry if last entry was same, or line was empty.
		if self.entries.back() == Some(&line) || line.is_empty() {
			return;
		}
		// Add entry to back of history
		self.entries.push_back(line);
		// Check if already have enough entries
		if self.entries.len() > self.max_size {
			// Remove oldest entry
			self.entries.pop_front();
		}
	}

	// Changes the history size.
	pub fn set_max_size(&mut self, max_size: usize) {
		self.max_size = max_size;

		while self.entries.len() > max_size {
			// Remove oldest entry
			self.entries.pop_front();
		}

		// Make sure we don't end up in an invalid position.
		self.reset_position();
	}

	// Returns the current history entries.
	pub fn get_entries(&self) -> &VecDeque<String> {
		&self.entries
	}

	// Replaces the current history entries.
	pub fn set_entries(&mut self, entries: impl IntoIterator<Item = String>) {
		self.entries.clear();

		// Using `add_entry` will respect `max_size` and remove duplicate lines etc.
		for entry in entries.into_iter() {
			self.add_entry(entry);
		}

		self.reset_position();
	}

	// Sets the history position back to the start.
	pub fn reset_position(&mut self) {
		self.current_position = None;
	}

	// Find next history that matches a given string from an index
	pub fn search_next(&mut self, _current: &str) -> Option<&str> {
		if let Some(index) = &mut self.current_position {
			if *index > 0 {
				*index -= 1;
			}
			Some(&self.entries[*index])
		} else if let Some(last) = self.entries.back() {
			self.current_position = Some(self.entries.len() - 1);
			Some(last)
		} else {
			None
		}
	}

	// Find previous history item that matches a given string from an index
	pub fn search_previous(&mut self, _current: &str) -> Option<&str> {
		if let Some(index) = &mut self.current_position {
			if *index == self.entries.len() - 1 {
				self.current_position = None;
				return Some("");
			}
			*index += 1;
			Some(&self.entries[*index])
		} else {
			None
		}
	}
}

#[cfg(test)]
#[test]
fn test_history() {
	let mut history = History::default();

	history.add_entry("foo".into());
	history.add_entry("bar".into());
	history.add_entry("baz".into());

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
#[test]
fn test_history_limit() {
	let mut history = History {
		max_size: 3,
		..Default::default()
	};

	history.add_entry("foo".into());
	history.add_entry("bar".into());
	history.add_entry("baz".into());
	history.add_entry("qux".into()); // Should remove "foo".

	assert_eq!(Some("qux"), history.search_next(""));
	assert_eq!(Some("baz"), history.search_next(""));
	assert_eq!(Some("bar"), history.search_next(""));
	assert_eq!(Some("bar"), history.search_next(""));

	history.set_max_size(2);

	assert_eq!(Some("qux"), history.search_next(""));
	assert_eq!(Some("baz"), history.search_next(""));
	assert_eq!(Some("baz"), history.search_next(""));
}

#[cfg(test)]
#[test]
fn test_history_reset_on_add() {
	let mut history = History::default();

	history.add_entry("foo".into());
	history.add_entry("bar".into());
	history.add_entry("baz".into());

	assert_eq!(None, history.search_previous(""));
	assert_eq!(Some("baz"), history.search_next(""));
	assert_eq!(Some("bar"), history.search_next(""));

	// This should reset the history position.
	history.add_entry("qux".into());

	assert_eq!(None, history.search_previous(""));
	assert_eq!(Some("qux"), history.search_next(""));
	assert_eq!(Some("baz"), history.search_next(""));
	assert_eq!(Some("bar"), history.search_next(""));
	assert_eq!(Some("foo"), history.search_next(""));
}

#[cfg(test)]
#[test]
fn test_history_export() {
	let mut history = History {
		max_size: 3,
		..Default::default()
	};

	assert_eq!(history.get_entries(), &VecDeque::new());

	history.add_entry("foo".into());
	history.add_entry("bar".into());
	history.add_entry("baz".into());

	assert_eq!(history.get_entries(), &["foo", "bar", "baz"]);

	history.add_entry("qux".into());

	assert_eq!(history.get_entries(), &["bar", "baz", "qux"]);

	history.set_entries(["a".to_string(), "b".to_string(), "b".to_string()]);

	assert_eq!(Some("b"), history.search_next(""));
	assert_eq!(Some("a"), history.search_next(""));
	assert_eq!(Some("a"), history.search_next(""));
}
