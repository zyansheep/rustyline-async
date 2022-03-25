#![feature(const_mut_refs)]
#![feature(try_blocks)]

use std::io::{self, Stdout, Write, stdout};

use futures::prelude::*;

use input_codec::{EventStream, event_stream};

use termion::{clear, cursor, event::{Event, Key}, raw::{IntoRawMode, RawTerminal}};
use thiserror::Error;

mod input_codec;

#[derive(Debug, Error)]
pub enum ReadlineError {
	#[error("io: {0}")]
	IO(#[from] io::Error),
	#[error("invalid utf8: {0}")]
	Utf(#[from] std::str::Utf8Error),
	#[error("end of file")]
	Eof,
	#[error("caught CTRL-C")]
	Interrupted,
}

#[derive(Default)]
pub struct LineState {
	line: String,
	cursor_pos: usize,
	prompt: String,
}


pub const CLEAR_AND_MOVE	: &str = "\x1b[2K\x1b[0E\x1b[0m";
pub const DELETE			: &str = "\x7f";
impl LineState {
	pub fn new(prompt: String) -> Self {
		Self { prompt, ..Default::default() }
	}
	fn clear(&self, term: &mut impl Write) -> io::Result<()> {
		write!(term, "{}{}", cursor::Left(1000), clear::CurrentLine)
	}
	fn clear_and_render(&self, term: &mut impl Write) -> io::Result<()> {
		self.clear(term)?;
		self.render(term)?;
		Ok(())
	}
	fn render(&self, term: &mut impl Write) -> io::Result<()> {
		write!(term, "{}{}", self.prompt, self.line)?;
		if self.cursor_pos < self.line.len() {
			write!(term, "{}", cursor::Left((self.line.len() - self.cursor_pos) as u16))?;
		}
		Ok(())
	}
	fn print_data(&self, data: &[u8], term: &mut impl Write) -> Result<(), ReadlineError> {
		self.clear(term)?;
		term.write(data)?;
		if !data.ends_with(&['\n' as u8]) { writeln!(term)?; }
		self.clear(term)?;
		self.render(term)?;
		Ok(())
	}
	fn print(&self, string: &str, term: &mut impl Write) -> Result<(), ReadlineError> {
		self.print_data(string.as_bytes(), term)?;
		Ok(())
	}
	fn handle_key(&mut self, key: Key, term: &mut impl Write) -> Result<Option<String>, ReadlineError> {
		// println!("key: {:?}", key);
		match key {
			// Return
			Key::Char('\n') => {
				let line = std::mem::replace(&mut self.line, String::new());
				self.cursor_pos = 0;
				self.clear_and_render(term)?;
				return Ok(Some(line))
			},
			// Delete character from line
			Key::Backspace => {
				if self.cursor_pos != 0 {
					self.cursor_pos = self.cursor_pos.saturating_sub(1);
					if self.cursor_pos == self.line.len() { // If at end of line
						let _ = self.line.pop();
					} else {
						self.line.remove(self.cursor_pos);
					}
					self.clear_and_render(term)?;
				}
				
			}
			// End of transmission (CTRL-D)
			Key::Ctrl('d') => {
				Err(ReadlineError::Eof)?
			}
			// End of text (CTRL-C)
			Key::Ctrl('c') => {
				self.print(&format!("{}{}", self.prompt, self.line), term)?;
				self.line.clear();
				self.cursor_pos = 0;
				self.clear_and_render(term)?;
			}
			Key::Left => {
				if self.cursor_pos > 0 {
					self.cursor_pos = self.cursor_pos.saturating_sub(1);
					write!(term, "{}", cursor::Left(1))?;
				}
			}
			Key::Right => {
				let new_pos = self.cursor_pos + 1;
				if new_pos <= self.line.len() {
					write!(term, "{}", cursor::Right(1))?;
					self.cursor_pos = new_pos;
				}
			}
			// Add character to line and output
			Key::Char(c) => {
				if self.cursor_pos == self.line.len() {
					self.line.push(c);
					self.cursor_pos += 1;
					write!(term, "{}", c)?;
				} else {
					self.cursor_pos += 1;
					self.line.insert(self.cursor_pos, c);
					self.clear_and_render(term)?;
				}
			},
			_ => {},
		}
		Ok(None)
	}
}

/// Structure that contains all the data necessary to read and write lines in a asyncronous manner
pub struct Readline<R: AsyncRead + Unpin> {
	raw_term: RawTerminal<Stdout>,
	event_stream: EventStream<R>, // Stream of events
	line: LineState, // Current line
}

impl<R: AsyncRead + Unpin> Readline<R> {
	pub fn new(prompt: String, reader: R) -> Result<Self, ReadlineError> {
		//let (stdout_reader, write) = sluice::pipe::pipe();
		let mut readline = Readline {
			//stdout_reader,
			raw_term: stdout().into_raw_mode()?,
			event_stream: event_stream(reader),
			line: LineState::new(prompt), // Current line state
		};
		readline.line.render(&mut readline.raw_term)?;
		readline.raw_term.flush()?;
		Ok(readline)
		
	}
	pub fn print(&mut self, string: &str) -> Result<(), ReadlineError> {
		self.line.print(string, &mut self.raw_term)
	}
	pub fn flush(&mut self) -> io::Result<()> {
		self.raw_term.flush()
	}
	pub async fn readline(&mut self) -> Option<Result<String, ReadlineError>> {
		// let out_buffer = [0u8; 1024]; // buffers data coming from external sources on its way to stdout
		let res: Result<String, ReadlineError> = try {
			match self.event_stream.next().await {
				Some(Ok(Event::Key(key))) => {
					match self.line.handle_key(key, &mut self.raw_term) {
						Ok(Some(line)) => Result::<_, ReadlineError>::Ok(line)?,
						Err(e) => Err(e)?,
						Ok(None) => return None,
					}
				}
				Some(Ok(_)) => return None,
				Some(Err(e)) => Err(e)?,
				None => return None,
			}
		};
		Some(res)
	}
}