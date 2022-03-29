#![feature(try_blocks)]
#![feature(io_error_other)]

use std::{
	io::{self, stdout, Stdout, Write},
	pin::Pin,
	task::{Context, Poll},
};

use futures::prelude::*;

use crossterm::{
	cursor,
	event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers},
	terminal::{self, disable_raw_mode, Clear, ClearType::*},
	QueueableCommand,
};
use thingbuf::mpsc::{errors::TrySendError, Receiver, Sender};
use thiserror::Error;

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
struct LineState {
	line: String,
	line_cursor_pos: usize,
	prompt: String,
	last_line_length: usize,
	last_line_completed: bool,

	term_size: (u16, u16),
}

impl LineState {
	fn new(prompt: String, term_size: (u16, u16)) -> Self {
		Self {
			prompt,
			last_line_completed: true,
			term_size,
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

	fn clear(&self, term: &mut impl Write) -> io::Result<()> {
		self.move_to_beginning(term, (self.prompt.len() + self.line_cursor_pos) as u16)?;
		term.queue(Clear(FromCursorDown))?;
		Ok(())
	}
	fn clear_and_render(&self, term: &mut impl Write) -> io::Result<()> {
		self.clear(term)?;
		self.render(term)?;
		Ok(())
	}
	fn render(&self, term: &mut impl Write) -> io::Result<()> {
		write!(term, "{}{}", self.prompt, self.line)?;
		self.move_to_beginning(term, (self.prompt.len() + self.line.len()) as u16)?;
		self.move_from_beginning(term, self.prompt.len() as u16 + self.line_cursor_pos as u16)?;
		Ok(())
	}
	fn print_data(&mut self, data: &[u8], term: &mut impl Write) -> Result<(), ReadlineError> {
		self.clear(term)?;

		// If last written data was not newline, restore the cursor
		if !self.last_line_completed {
			term.queue(cursor::MoveUp(1))?
				.queue(cursor::MoveToColumn(1))?
				.queue(cursor::MoveRight(self.last_line_length as u16))?;
			// term.queue(cursor::RestorePosition)?; // Move cursor to previous line
		}
		// Write data
		for line in data.split_inclusive(|b| *b == b'\n') {
			term.write_all(line)?;
			term.write_all(b"\x1b[1000D")?;
		}
		// write!(term, "{:X?}", data)?;
		self.last_line_completed = data.ends_with(b"\n"); // Set whether data ends with newline

		// If data does not end with newline, save the cursor and write newline for prompt
		if !self.last_line_completed {
			self.last_line_length += data.len();
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
	fn handle_event(
		&mut self,
		event: Event,
		term: &mut impl Write,
	) -> Result<Option<String>, ReadlineError> {
		// println!("key: {:?}", key);
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
					self.line_cursor_pos = 0;
					self.render(term)?;
					return Ok(Some(line));
				}
				// Delete character from line
				KeyCode::Backspace => {
					if self.line_cursor_pos != 0 {
						self.clear(term)?;
						self.line_cursor_pos = self.line_cursor_pos.saturating_sub(1);
						if self.line_cursor_pos == self.line.len() {
							// If at end of line
							let _ = self.line.pop();
						} else {
							self.line.remove(self.line_cursor_pos);
						}
						self.render(term)?;
					}
				}
				KeyCode::Left => {
					if self.line_cursor_pos > 0 {
						self.line_cursor_pos = self.line_cursor_pos.saturating_sub(1);
						term.queue(cursor::MoveLeft(1))?;
					}
				}
				KeyCode::Right => {
					let new_pos = self.line_cursor_pos + 1;
					if new_pos <= self.line.len() {
						term.queue(cursor::MoveRight(1))?;
						self.line_cursor_pos = new_pos;
					}
				}
				// Add character to line and output
				KeyCode::Char(c) => {
					self.clear(term)?;
					self.line_cursor_pos += 1;
					if self.line_cursor_pos == self.line.len() {
						self.line.push(c);
					} else {
						self.line.insert(self.line_cursor_pos - 1, c);
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
					self.line_cursor_pos = 0;
					self.clear_and_render(term)?;
					return Err(ReadlineError::Interrupted);
				}
				KeyCode::Char('l') => {
					term.queue(Clear(All))?.queue(cursor::MoveToColumn(0))?;
					self.clear_and_render(term)?;
				}
				KeyCode::Char('u') => {
					self.clear(term)?;
					self.line.drain(0..self.line_cursor_pos);
					self.line_cursor_pos = 0;
					term.queue(cursor::MoveDown(self.line_height(
						((self.prompt.len() + self.line.len()) - self.line_cursor_pos) as u16,
					)))?;
					self.render(term)?;
				}
				KeyCode::Left => {
					self.clear(term)?;
					self.line_cursor_pos = if let Some((new_pos, _)) = self.line
						[0..self.line_cursor_pos]
						.char_indices()
						.rev()
						.skip_while(|(_, c)| *c == ' ')
						.find(|(_, c)| *c == ' ')
					{
						new_pos + 1
					} else {
						0
					};

					self.render(term)?;
				}
				KeyCode::Right => {
					self.clear(term)?;
					self.line_cursor_pos = if let Some((new_pos, _)) = self.line
						[self.line_cursor_pos..self.line.len()]
						.char_indices()
						.skip_while(|(_, c)| *c == ' ')
						.find(|(_, c)| *c == ' ')
					{
						self.line_cursor_pos + new_pos
					} else {
						self.line.len()
					};
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

/// Clonable object that implements `Write` and `AsyncWrite` and allows for sending data to the output without messing up the readline.
#[derive(Clone)]
pub struct SharedWriter {
	sender: Sender<Vec<u8>>,
}
impl AsyncWrite for SharedWriter {
	fn poll_write(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<io::Result<usize>> {
		let fut = self.sender.send_ref();
		futures::pin_mut!(fut);
		let mut send_buf = futures::ready!(fut.poll_unpin(cx))
			.map_err(|_| io::Error::other("thingbuf receiver has closed"))?;
		send_buf.extend_from_slice(buf);
		Poll::Ready(Ok(buf.len()))
	}
	fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
		Poll::Ready(Ok(()))
	}
	fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
		Poll::Ready(Ok(()))
	}
}
impl io::Write for SharedWriter {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		match self.sender.try_send_ref() {
			Ok(mut send_buf) => {
				send_buf.extend_from_slice(buf);
				Ok(buf.len())
			}
			Err(TrySendError::Full(_)) => Err(io::ErrorKind::WouldBlock.into()),
			_ => Err(io::Error::other("thingbuf receiver has closed")),
		}
	}
	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}

/// Structure that contains all the data necessary to read and write lines in an asyncronous manner
pub struct Readline {
	raw_term: Stdout,
	event_stream: EventStream, // Stream of events
	line_receiver: Receiver<Vec<u8>>,

	line: LineState, // Current line
}

impl Readline {
	pub fn new(prompt: String) -> Result<(Self, SharedWriter), ReadlineError> {
		let (sender, line_receiver) = thingbuf::mpsc::channel(100);
		terminal::enable_raw_mode()?;
		let mut readline = Readline {
			raw_term: stdout(),
			event_stream: EventStream::new(),
			line_receiver,
			line: LineState::new(prompt, terminal::size()?),
		};
		readline.line.render(&mut readline.raw_term)?;
		readline.raw_term.queue(terminal::EnableLineWrap)?;
		readline.raw_term.flush()?;
		Ok((readline, SharedWriter { sender }))
	}
	pub fn flush(&mut self) -> io::Result<()> {
		self.raw_term.flush()
	}
	pub async fn readline(&mut self) -> Option<Result<String, ReadlineError>> {
		let res: Result<String, ReadlineError> = try {
			futures::select! {
				event = self.event_stream.next().fuse() => match event {
					Some(Ok(event)) => {
						match self.line.handle_event(event, &mut self.raw_term) {
							Ok(Some(line)) => Result::<_, ReadlineError>::Ok(line)?,
							Err(e) => Err(e)?,
							Ok(None) => { self.raw_term.flush()?; return None },
						}
					}
					Some(Err(e)) => Err(e)?,
					None => return None,
				},
				result = self.line_receiver.recv_ref().fuse() => match result {
					Some(buf) => {
						self.line.print_data(&buf, &mut self.raw_term)?;
						self.raw_term.flush()?;
						return None
					},
					None => Err(ReadlineError::Closed)?,
				}
			}
		};
		Some(res)
	}
}

impl Drop for Readline {
	fn drop(&mut self) {
		let _ = disable_raw_mode();
	}
}
