use std::{
	io::{self, stdout, Stdout, Write},
	ops::DerefMut,
	pin::Pin,
	task::{Context, Poll},
};
use std::cmp::min;
use std::collections::VecDeque;
use std::sync::Mutex;

use crossterm::{
	cursor,
	event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers},
	QueueableCommand,
	terminal::{self, Clear, ClearType::*, disable_raw_mode},
};
use futures::prelude::*;
use thingbuf::mpsc::{errors::TrySendError, Receiver, Sender};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Error returned from `readline()`
#[derive(Debug, Error)]
pub enum ReadlineError {
	#[error("io: {0}")]
	IO(#[from] io::Error),
	#[error("end of file")]
	Eof,
	#[error("caught CTRL-C")]
	Interrupted,
	#[error("line writers closed")]
	Closed,
}

#[derive(Default)]
struct History {
	entries: VecDeque<String>,
	max_size: usize,
	// The current offset in the history (1-indexed). 0 indicates a fresh line.
	offset: usize,
}

impl History {
	fn new(max_size: usize) -> History {
		History {
			max_size,

			..Default::default()
		}
	}

	fn len(&self) -> usize {
		self.entries.len()
	}

	fn push(&mut self, entry: String) {
		if self.entries.len() >= self.max_size {
			self.entries.pop_back();
		}

		if self.offset != 0 {
			self.offset = min(self.offset + 1, self.max_size);
		}
		self.entries.push_front(entry);
	}

	fn current(&self) -> Option<&String> {
		self.entries.get(self.offset - 1)
	}
}

#[derive(Default)]
struct LineState {
	// Unicode Line
	line: String,
	// Index of grapheme in line
	line_cursor_grapheme: usize,
	// Column of grapheme in line
	current_column: u16,

	cluster_buffer: String, // buffer for holding partial grapheme clusters as they come in

	prompt: String,
	last_line_length: usize,
	last_line_completed: bool,

	term_size: (u16, u16),

	history: Option<Mutex<History>>,
}

impl LineState {
	fn new(prompt: String, term_size: (u16, u16), max_history_size: usize) -> Self {
		let current_column = prompt.len() as u16;
		Self {
			prompt,
			last_line_completed: true,
			term_size,
			current_column,

			history: if max_history_size == 0 {
				None
			} else {
				Some(Mutex::new(History::new(max_history_size)))
			},

			..Default::default()
		}
	}
	fn line_height(&self, pos: u16) -> u16 {
		pos / self.term_size.0 // Gets the number of lines wrapped
	}
	/// Move from a position on the line to the start
	fn move_to_beginning(&self, term: &mut impl Write, from: u16) -> io::Result<()> {
		let move_up = self.line_height(from.saturating_sub(1));
		term.queue(cursor::MoveToColumn(1))?
			.queue(cursor::MoveUp(move_up))?;
		Ok(())
	}
	/// Move from the start of the line to some position
	fn move_from_beginning(&self, term: &mut impl Write, to: u16) -> io::Result<()> {
		let line_height = self.line_height(to.saturating_sub(1));
		let line_remaining_len = to % self.term_size.0; // Get the remaining length
		term.queue(cursor::MoveDown(line_height))?
			.queue(cursor::MoveRight(line_remaining_len))?;
		Ok(())
	}
	fn move_cursor(&mut self, change: isize) -> io::Result<()> {
		// self.reset_cursor(term)?;
		if change > 0 {
			let count = self.line.graphemes(true).count();
			self.line_cursor_grapheme =
				usize::min(self.line_cursor_grapheme as usize + change as usize, count);
		} else {
			self.line_cursor_grapheme =
				self.line_cursor_grapheme.saturating_sub((-change) as usize);
		}
		let (pos, str) = self.current_grapheme().unwrap_or((0, ""));
		let pos = pos + str.len();
		self.current_column =
			(self.prompt.len() + UnicodeWidthStr::width(&self.line[0..pos])) as u16;

		// self.set_cursor(term)?;

		Ok(())
	}
	fn current_grapheme(&self) -> Option<(usize, &str)> {
		self.line
			.grapheme_indices(true)
			.take(self.line_cursor_grapheme)
			.last()
	}
	fn reset_cursor(&self, term: &mut impl Write) -> io::Result<()> {
		self.move_to_beginning(term, self.current_column)
	}
	fn set_cursor(&self, term: &mut impl Write) -> io::Result<()> {
		self.move_from_beginning(term, self.current_column as u16)
	}
	/// Clear current line
	fn clear(&self, term: &mut impl Write) -> io::Result<()> {
		self.move_to_beginning(term, self.current_column as u16)?;
		term.queue(Clear(FromCursorDown))?;
		Ok(())
	}
	/// Render line
	fn render(&self, term: &mut impl Write) -> io::Result<()> {
		write!(term, "{}{}", self.prompt, self.line)?;
		let line_len = self.prompt.len() + UnicodeWidthStr::width(&self.line[..]);
		self.move_to_beginning(term, line_len as u16)?;
		self.move_from_beginning(term, self.current_column)?;
		Ok(())
	}
	/// Clear line and render
	fn clear_and_render(&self, term: &mut impl Write) -> io::Result<()> {
		self.clear(term)?;
		self.render(term)?;
		Ok(())
	}
	fn print_data(&mut self, data: &[u8], term: &mut impl Write) -> Result<(), ReadlineError> {
		self.clear(term)?;

		// If last written data was not newline, restore the cursor
		if !self.last_line_completed {
			term.queue(cursor::MoveUp(1))?
				.queue(cursor::MoveToColumn(1))?
				.queue(cursor::MoveRight(self.last_line_length as u16))?;
		}

		// Write data in a way that newlines also act as carriage returns
		for line in data.split_inclusive(|b| *b == b'\n') {
			term.write_all(line)?;
			term.queue(cursor::MoveToColumn(1))?;
		}

		self.last_line_completed = data.ends_with(b"\n"); // Set whether data ends with newline

		// If data does not end with newline, save the cursor and write newline for prompt
		// Usually data does end in newline due to the buffering of SharedWriter, but sometimes it may not (i.e. if .flush() is called)
		if !self.last_line_completed {
			self.last_line_length += data.len();
			// Make sure that last_line_length wraps around when doing multiple writes
			if self.last_line_length >= self.term_size.0 as usize {
				self.last_line_length %= self.term_size.0 as usize;
				writeln!(term)?;
			}
			writeln!(term)?; // Move to beginning of line and make new line
		} else {
			self.last_line_length = 0;
		}

		term.queue(cursor::MoveToColumn(1))?;

		self.render(term)?;
		Ok(())
	}
	fn print(&mut self, string: &str, term: &mut impl Write) -> Result<(), ReadlineError> {
		self.print_data(string.as_bytes(), term)?;
		Ok(())
	}
	fn add_history_entry(&self, entry: String) {
		if let Some(ref history) = self.history {
			history.lock().expect("Failed to acquire lock on history").push(entry);
		}
	}
	fn handle_event(
		&mut self,
		event: Event,
		term: &mut impl Write,
	) -> Result<Option<String>, ReadlineError> {
		match event {
			// Regular Modifiers (None or Shift)
			Event::Key(KeyEvent {
				code,
				modifiers: KeyModifiers::NONE,
			})
			| Event::Key(KeyEvent {
				code,
				modifiers: KeyModifiers::SHIFT,
			}) => match code {
				KeyCode::Enter => {
					self.clear(term)?;
					let line = std::mem::take(&mut self.line);
					self.move_cursor(-100000)?;
					self.render(term)?;
					if let Some(ref history) = self.history {
						history.lock().expect("Failed to acquire lock on history").offset = 0;
					}
					return Ok(Some(line));
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
				KeyCode::Up => {
					if let Some(ref history) = self.history {
						let mut history = history.lock().expect("Failed to acquire lock on history");
						if history.offset < history.len() {
							history.offset += 1;
							self.line = history.current().map(|s| s.clone()).unwrap_or_default();
							drop(history); // Unlock history

							self.clear(term)?;
							self.move_cursor(100000)?;
							self.render(term)?;
						}
					}
				}
				KeyCode::Down => {
					if let Some(ref history) = self.history {
						let mut history = history.lock().expect("Failed to acquire lock on history");
						if history.offset > 0 {
							history.offset -= 1;

							if history.offset > 0 {
								self.line = history.current().map(|s| s.clone()).unwrap_or_default();
								drop(history); // Unlock history
								self.clear(term)?;
								self.move_cursor(100000)?;
							} else {
								self.line.clear();
								drop(history); // Unlock history
								self.clear(term)?;
								self.move_cursor(-100000)?;
							}
							self.render(term)?;
						}
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
			// Control Keys
			Event::Key(KeyEvent {
				code,
				modifiers: KeyModifiers::CONTROL,
			}) => match code {
				// End of transmission (CTRL-D)
				KeyCode::Char('d') => {
					writeln!(term)?;
					self.clear(term)?;
					return Err(ReadlineError::Eof);
				}
				// End of text (CTRL-C)
				KeyCode::Char('c') => {
					self.print(&format!("{}{}", self.prompt, self.line), term)?;
					self.line.clear();
					self.move_cursor(-10000)?;
					self.clear_and_render(term)?;
					return Err(ReadlineError::Interrupted);
				}
				KeyCode::Char('l') => {
					term.queue(Clear(All))?.queue(cursor::MoveToColumn(0))?;
					self.clear_and_render(term)?;
				}
				KeyCode::Char('u') => {
					if let Some((pos, str)) = self.current_grapheme() {
						let pos = pos + str.len();
						self.line.drain(0..pos);
						self.move_cursor(-10000)?;
						self.clear_and_render(term)?;
					}
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
						self.move_cursor(-10000)?
					}
					self.set_cursor(term)?;
				}
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
			Event::Resize(x, y) => {
				self.term_size = (x, y);
				self.clear_and_render(term)?;
			}
			_ => {}
		}
		Ok(None)
	}
}

/// Clonable object that implements `Write` and `AsyncWrite` and allows for sending data to the output without messing up the readline.
#[pin_project::pin_project]
pub struct SharedWriter {
	#[pin]
	buffer: Vec<u8>,
	sender: Sender<Vec<u8>>,
}
impl Clone for SharedWriter {
	fn clone(&self) -> Self {
		Self {
			buffer: Vec::new(),
			sender: self.sender.clone(),
		}
	}
}
impl AsyncWrite for SharedWriter {
	fn poll_write(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<io::Result<usize>> {
		let mut this = self.project();
		this.buffer.extend_from_slice(buf);
		if this.buffer.ends_with(b"\n") {
			let fut = this.sender.send_ref();
			futures::pin_mut!(fut);
			let mut send_buf = futures::ready!(fut.poll_unpin(cx)).map_err(|_| {
				io::Error::new(io::ErrorKind::Other, "thingbuf receiver has closed")
			})?;
			// Swap buffers
			std::mem::swap(send_buf.deref_mut(), &mut this.buffer);
			this.buffer.clear();
			Poll::Ready(Ok(buf.len()))
		} else {
			Poll::Ready(Ok(buf.len()))
		}
	}
	fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
		let mut this = self.project();
		let fut = this.sender.send_ref();
		futures::pin_mut!(fut);
		let mut send_buf = futures::ready!(fut.poll_unpin(cx))
			.map_err(|_| io::Error::new(io::ErrorKind::Other, "thingbuf receiver has closed"))?;
		// Swap buffers
		std::mem::swap(send_buf.deref_mut(), &mut this.buffer);
		this.buffer.clear();
		Poll::Ready(Ok(()))
	}
	fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
		Poll::Ready(Ok(()))
	}
}
impl io::Write for SharedWriter {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		self.buffer.extend_from_slice(buf);
		if self.buffer.ends_with(b"\n") {
			match self.sender.try_send_ref() {
				Ok(mut send_buf) => {
					std::mem::swap(send_buf.deref_mut(), &mut self.buffer);
					self.buffer.clear();
				}
				Err(TrySendError::Full(_)) => return Err(io::ErrorKind::WouldBlock.into()),
				_ => {
					return Err(io::Error::new(
						io::ErrorKind::Other,
						"thingbuf receiver has closed",
					));
				}
			}
		}
		Ok(buf.len())
	}
	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}

/// Structure that contains all the data necessary to read and write lines in an asyncronous manner
pub struct Readline {
	raw_term: Stdout,
	event_stream: EventStream,
	// Stream of events
	line_receiver: Receiver<Vec<u8>>,

	line: LineState, // Current line
}

impl Readline {
	pub fn new(prompt: String) -> Result<(Self, SharedWriter), ReadlineError> {
		Self::with_history(prompt, 0)
	}
	pub fn with_history(prompt: String, max_history_size: usize) -> Result<(Self, SharedWriter), ReadlineError> {
		let (sender, line_receiver) = thingbuf::mpsc::channel(500);
		terminal::enable_raw_mode()?;
		let mut readline = Readline {
			raw_term: stdout(),
			event_stream: EventStream::new(),
			line_receiver,
			line: LineState::new(prompt, terminal::size()?, max_history_size),
		};
		readline.line.render(&mut readline.raw_term)?;
		readline.raw_term.queue(terminal::EnableLineWrap)?;
		readline.raw_term.flush()?;
		Ok((
			readline,
			SharedWriter {
				sender,
				buffer: Vec::new(),
			},
		))
	}
	pub fn flush(&mut self) -> io::Result<()> {
		self.raw_term.flush()
	}
	pub async fn readline(&mut self) -> Result<String, ReadlineError> {
		loop {
			futures::select! {
				event = self.event_stream.next().fuse() => match event {
					Some(Ok(event)) => {
						match self.line.handle_event(event, &mut self.raw_term) {
							Ok(Some(line)) => return Result::<_, ReadlineError>::Ok(line),
							Err(e) => return Err(e),
							Ok(None) => self.raw_term.flush()?,
						}
					}
					Some(Err(e)) => return Err(e.into()),
					None => {},
				},
				result = self.line_receiver.recv_ref().fuse() => match result {
					Some(buf) => {
						self.line.print_data(&buf, &mut self.raw_term)?;
						self.raw_term.flush()?;
					},
					None => return Err(ReadlineError::Closed),
				}
			}
		}
	}
	pub fn add_history_entry(&self, entry: String) {
		self.line.add_history_entry(entry);
	}
}

impl Drop for Readline {
	fn drop(&mut self) {
		let _ = disable_raw_mode();
	}
}
