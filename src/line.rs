use std::io::{self, Write};

use crossterm::{
	cursor,
	event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
	terminal::{Clear, ClearType::*},
	QueueableCommand,
};

use ansi_width::ansi_width;
use unicode_segmentation::UnicodeSegmentation;

use crate::{History, ReadlineError, ReadlineEvent};

#[derive(Default)]
pub struct LineState {
	// Unicode Line
	line: String,
	// Index of grapheme in line
	line_cursor_grapheme: usize,
	// Column of grapheme in line
	current_column: u16,

	cluster_buffer: String, // buffer for holding partial grapheme clusters as they come in

	prompt: String,
	prompt_len: u16,
	pub should_print_line_on_enter: bool, // After pressing enter, should we print the line just submitted?
	pub should_print_line_on_control_c: bool, // After pressing control_c should we print the line just cancelled?

	last_line_length: usize,
	last_line_completed: bool,

	term_size: (u16, u16),

	pub history: History,
}

impl LineState {
	pub fn new(prompt: String, term_size: (u16, u16)) -> Self {
		let prompt_len = ansi_width(&prompt) as u16;
		Self {
			prompt,
			prompt_len,
			last_line_completed: true,
			term_size: (term_size.0.max(1), term_size.1),
			current_column: prompt_len,
			should_print_line_on_enter: true,
			should_print_line_on_control_c: true,

			..Default::default()
		}
	}
	fn line_height(&self, pos: u16) -> u16 {
		pos / self.term_size.0 // Gets the number of lines wrapped
	}
	/// Move from a position on the line to the start
	fn move_to_beginning(&self, term: &mut impl Write, from: u16) -> io::Result<()> {
		let move_up = self.line_height(from.saturating_sub(1));
		term.queue(cursor::MoveToColumn(0))?;
		if move_up != 0 {
			term.queue(cursor::MoveUp(move_up))?;
		}
		Ok(())
	}
	/// Move from the start of the line to some position
	fn move_from_beginning(&self, term: &mut impl Write, to: u16) -> io::Result<()> {
		let line_height = self.line_height(to.saturating_sub(1));
		let line_remaining_len = to % self.term_size.0; // Get the remaining length
		if line_height != 0 {
			term.queue(cursor::MoveDown(line_height))?;
		}
		// Use absolute positioning to prevent cursor drift with async output
		term.queue(cursor::MoveToColumn(line_remaining_len))?;

		Ok(())
	}
	/// Move cursor by one unicode grapheme either left (negative) or right (positive)
	fn move_cursor(&mut self, change: isize) -> io::Result<()> {
		if change > 0 {
			let count = self.line.graphemes(true).count();
			self.line_cursor_grapheme =
				usize::min(self.line_cursor_grapheme + change as usize, count);
		} else {
			self.line_cursor_grapheme =
				self.line_cursor_grapheme.saturating_sub((-change) as usize);
		}
		let (pos, str) = self.current_grapheme().unwrap_or((0, ""));
		let pos = pos + str.len();
		self.current_column = self.prompt_len + ansi_width(&self.line[0..pos]) as u16;

		Ok(())
	}
	fn current_grapheme(&self) -> Option<(usize, &str)> {
		self.line
			.grapheme_indices(true)
			.take(self.line_cursor_grapheme)
			.last()
	}
	fn next_grapheme(&self) -> Option<(usize, &str)> {
		let total = self.line.grapheme_indices(true).count();
		if self.line_cursor_grapheme == total {
			return None;
		}
		self.line
			.grapheme_indices(true)
			.take(self.line_cursor_grapheme + 1)
			.last()
	}
	fn reset_cursor(&self, term: &mut impl Write) -> io::Result<()> {
		self.move_to_beginning(term, self.current_column)
	}
	fn set_cursor(&self, term: &mut impl Write) -> io::Result<()> {
		self.move_from_beginning(term, self.current_column)
	}
	/// Clear current line
	pub fn clear(&self, term: &mut impl Write) -> io::Result<()> {
		self.move_to_beginning(term, self.current_column)?;
		term.queue(Clear(FromCursorDown))?;
		Ok(())
	}
	/// Render line
	pub fn render(&self, term: &mut impl Write) -> io::Result<()> {
		write!(term, "{}{}", self.prompt, self.line)?;
		let line_len = self.prompt_len + ansi_width(&self.line[..]) as u16;
		self.move_to_beginning(term, line_len)?;
		self.move_from_beginning(term, self.current_column)?;
		Ok(())
	}
	/// Clear line and render
	pub fn clear_and_render(&self, term: &mut impl Write) -> io::Result<()> {
		self.clear(term)?;
		self.render(term)?;
		Ok(())
	}
	pub fn print_data(&mut self, data: &[u8], term: &mut impl Write) -> Result<(), ReadlineError> {
		self.clear(term)?;

		// If last written data was not newline, restore the cursor
		if !self.last_line_completed {
			term.queue(cursor::MoveUp(1))?
				.queue(cursor::MoveToColumn(0))?
				.queue(cursor::MoveRight(self.last_line_length as u16))?;
		}

		// Write data in a way that newlines also act as carriage returns
		for line in data.split_inclusive(|b| *b == b'\n') {
			term.write_all(line)?;
			term.queue(cursor::MoveToColumn(0))?;
		}

		self.last_line_completed = data.ends_with(b"\n"); // Set whether data ends with newline

		// If data does not end with newline, save the cursor and write newline for prompt
		// Usually data does end in newline due to the buffering of SharedWriter, but sometimes it may not (i.e. if .flush() is called)
		if !self.last_line_completed {
			self.last_line_length += ansi_width(&String::from_utf8_lossy(data));
			// Make sure that last_line_length wraps around when doing multiple writes
			if self.last_line_length >= self.term_size.0 as usize {
				self.last_line_length %= self.term_size.0 as usize;
				writeln!(term)?;
			}
			writeln!(term)?; // Move to beginning of line and make new line
		} else {
			self.last_line_length = 0;
		}

		term.queue(cursor::MoveToColumn(0))?;

		self.render(term)?;
		Ok(())
	}
	pub fn print(&mut self, string: &str, term: &mut impl Write) -> Result<(), ReadlineError> {
		self.print_data(string.as_bytes(), term)?;
		Ok(())
	}
	pub fn update_prompt(
		&mut self,
		prompt: &str,
		term: &mut impl Write,
	) -> Result<(), ReadlineError> {
		self.clear(term)?;
		self.prompt.clear();
		self.prompt.push_str(prompt);
		self.prompt_len = ansi_width(&self.prompt) as u16;
		// recalculates column
		self.move_cursor(0)?;
		self.render(term)?;
		term.flush()?;
		Ok(())
	}
	pub fn handle_event(
		&mut self,
		event: Event,
		term: &mut impl Write,
	) -> Result<Option<ReadlineEvent>, ReadlineError> {
		match event {
			// Control Keys
			Event::Key(KeyEvent {
				code,
				modifiers: KeyModifiers::CONTROL,
				kind: KeyEventKind::Press,
				..
			}) => match code {
				// End of transmission (CTRL-D)
				KeyCode::Char('d') => {
					writeln!(term)?;
					self.clear(term)?;
					return Ok(Some(ReadlineEvent::Eof));
				}
				// End of text (CTRL-C)
				KeyCode::Char('c') => {
					if self.should_print_line_on_control_c {
						self.print(&format!("{}{}", self.prompt, self.line), term)?;
					}

					self.line.clear();
					self.move_cursor(-10000)?;
					self.clear_and_render(term)?;
					return Ok(Some(ReadlineEvent::Interrupted));
				}
				// Clear all
				KeyCode::Char('l') => {
					term.queue(Clear(All))?.queue(cursor::MoveTo(0, 0))?;
					self.clear_and_render(term)?;
				}
				// Clear to start
				KeyCode::Char('u') => {
					if let Some((pos, str)) = self.current_grapheme() {
						let pos = pos + str.len();
						self.line.drain(0..pos);
						self.move_cursor(-100000)?;
						self.clear_and_render(term)?;
					}
				}
				// Clear last word
				KeyCode::Char('w') => {
					let count = self.line.graphemes(true).count();
					let skip_count = count - self.line_cursor_grapheme;
					let start = self
						.line
						.grapheme_indices(true)
						.rev()
						.skip(skip_count)
						.skip_while(|(_, str)| *str == " ")
						.find_map(|(pos, str)| if str == " " { Some(pos + 1) } else { None })
						.unwrap_or(0);
					let end = self
						.line
						.grapheme_indices(true)
						.nth(self.line_cursor_grapheme)
						.map(|(end, _)| end);
					let change = start as isize - self.line_cursor_grapheme as isize;
					self.move_cursor(change)?;
					if let Some(end) = end {
						self.line.drain(start..end);
					} else {
						self.line.drain(start..);
					}
					self.clear_and_render(term)?;
				}
				// Move to beginning
				#[cfg(feature = "emacs")]
				KeyCode::Char('a') => {
					self.reset_cursor(term)?;
					self.move_cursor(-100000)?;
					self.set_cursor(term)?;
				}
				// Move to end
				#[cfg(feature = "emacs")]
				KeyCode::Char('e') => {
					self.reset_cursor(term)?;
					self.move_cursor(100000)?;
					self.set_cursor(term)?;
				}
				// Move cursor left to previous word
				KeyCode::Left => {
					self.reset_cursor(term)?;
					let count = self.line.graphemes(true).count();
					let skip_count = count - self.line_cursor_grapheme;
					if let Some((pos, _)) = self
						.line
						.grapheme_indices(true)
						.rev()
						.skip(skip_count)
						.skip_while(|(_, str)| *str == " ")
						.find(|(_, str)| *str == " ")
					{
						let change = pos as isize - self.line_cursor_grapheme as isize;
						self.move_cursor(change + 1)?;
					} else {
						self.move_cursor(-100000)?
					}
					self.set_cursor(term)?;
				}
				// Move cursor right to next word
				KeyCode::Right => {
					self.reset_cursor(term)?;
					if let Some((pos, _)) = self
						.line
						.grapheme_indices(true)
						.skip(self.line_cursor_grapheme)
						.skip_while(|(_, c)| *c == " ")
						.find(|(_, c)| *c == " ")
					{
						let change = pos as isize - self.line_cursor_grapheme as isize;
						self.move_cursor(change)?;
					} else {
						self.move_cursor(10000)?;
					};
					self.set_cursor(term)?;
				}
				_ => {}
			},
			// Other Modifiers (None, Shift, Control+Alt)
			// All other modifiers must be considered because the match expression cannot match
			// combined KeyModifiers. Control+Alt is used to reach certain special symbols on a lot
			// of international keyboard layouts.
			Event::Key(KeyEvent {
				code,
				modifiers: _,
				kind: KeyEventKind::Press,
				..
			}) => match code {
				KeyCode::Enter => {
					// Print line so you can see what commands you've typed
					if self.should_print_line_on_enter {
						self.print(&format!("{}{}\n", self.prompt, self.line), term)?;
					}

					// Take line
					let line = std::mem::take(&mut self.line);

					// Render new line from beginning
					self.move_cursor(-100000)?;
					self.clear_and_render(term)?;
					self.history.reset_position();

					// Return line
					return Ok(Some(ReadlineEvent::Line(line)));
				}
				// Delete character from line
				KeyCode::Backspace => {
					if let Some((pos, str)) = self.current_grapheme() {
						self.clear(term)?;

						let len = pos + str.len();
						self.line.replace_range(pos..len, "");
						self.move_cursor(-1)?;

						self.render(term)?;
					}
				}
				KeyCode::Delete => {
					if let Some((pos, str)) = self.next_grapheme() {
						self.clear(term)?;

						let len = pos + str.len();
						self.line.replace_range(pos..len, "");

						self.render(term)?;
					}
				}
				KeyCode::Left => {
					self.reset_cursor(term)?;
					self.move_cursor(-1)?;
					self.set_cursor(term)?;
				}
				KeyCode::Right => {
					self.reset_cursor(term)?;
					self.move_cursor(1)?;
					self.set_cursor(term)?;
				}
				KeyCode::Home => {
					self.reset_cursor(term)?;
					self.move_cursor(-100000)?;
					self.set_cursor(term)?;
				}
				KeyCode::End => {
					self.reset_cursor(term)?;
					self.move_cursor(100000)?;
					self.set_cursor(term)?;
				}
				KeyCode::Up => {
					// search for next history item, replace line if found.
					if let Some(line) = self.history.search_next(&self.line) {
						self.line.clear();
						self.line += line;
						self.clear(term)?;
						self.move_cursor(100000)?;
						self.render(term)?;
					}
				}
				KeyCode::Down => {
					// search for next history item, replace line if found.
					if let Some(line) = self.history.search_previous(&self.line) {
						self.line.clear();
						self.line += line;
						self.clear(term)?;
						self.move_cursor(100000)?;
						self.render(term)?;
					}
				}
				// Add character to line and output
				KeyCode::Char(c) => {
					self.clear(term)?;
					let prev_len = self.cluster_buffer.graphemes(true).count();
					self.cluster_buffer.push(c);
					let new_len = self.cluster_buffer.graphemes(true).count();

					let (g_pos, g_str) = self.current_grapheme().unwrap_or((0, ""));
					let pos = g_pos + g_str.len();

					self.line.insert(pos, c);

					if prev_len != new_len {
						self.move_cursor(1)?;
						if prev_len > 0 {
							if let Some((pos, str)) =
								self.cluster_buffer.grapheme_indices(true).next()
							{
								let len = str.len();
								self.cluster_buffer.replace_range(pos..len, "");
							}
						}
					}
					self.render(term)?;
				}
				_ => {}
			},
			Event::Resize(x, y) => {
				self.term_size = (x, y);
				self.clear_and_render(term)?;
			}
			_ => {}
		}
		Ok(None)
	}
}
