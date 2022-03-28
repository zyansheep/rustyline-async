#![feature(try_blocks)]
#![feature(io_error_other)]

use std::{
	io::{self, stdout, Stdout, Write},
	pin::Pin,
	task::{Context, Poll},
};

use futures::prelude::*;

use input_codec::{event_stream, EventStream};

use termion::{
	clear, cursor,
	event::{Event, Key},
	raw::{IntoRawMode, RawTerminal},
};
use thingbuf::mpsc::{errors::TrySendError, Receiver, Sender};
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
	#[error("line writers closed")]
	Closed,
}

#[derive(Default)]
pub struct LineState {
	line: String,
	cursor_pos: usize,
	prompt: String,
}

pub const CLEAR_AND_MOVE: &str = "\x1b[2K\x1b[0E\x1b[0m";
pub const DELETE: &str = "\x7f";
impl LineState {
	pub fn new(prompt: String) -> Self {
		Self {
			prompt,
			..Default::default()
		}
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
			write!(
				term,
				"{}",
				cursor::Left((self.line.len() - self.cursor_pos) as u16)
			)?;
		}
		Ok(())
	}
	fn print_data(&self, data: &[u8], term: &mut impl Write) -> Result<(), ReadlineError> {
		self.clear(term)?;
		term.write(data)?;
		if !data.ends_with(&['\n' as u8]) {
			write!(term, "\n")?;
		}
		self.clear(term)?;
		self.render(term)?;
		Ok(())
	}
	fn print(&self, string: &str, term: &mut impl Write) -> Result<(), ReadlineError> {
		self.print_data(string.as_bytes(), term)?;
		Ok(())
	}
	fn handle_key(
		&mut self,
		key: Key,
		term: &mut impl Write,
	) -> Result<Option<String>, ReadlineError> {
		// println!("key: {:?}", key);
		match key {
			// Return
			Key::Char('\n') => {
				let line = std::mem::replace(&mut self.line, String::new());
				self.cursor_pos = 0;
				self.clear_and_render(term)?;
				return Ok(Some(line));
			}
			// Delete character from line
			Key::Backspace => {
				if self.cursor_pos != 0 {
					self.cursor_pos = self.cursor_pos.saturating_sub(1);
					if self.cursor_pos == self.line.len() {
						// If at end of line
						let _ = self.line.pop();
					} else {
						self.line.remove(self.cursor_pos);
					}
					self.clear_and_render(term)?;
				}
			}
			// End of transmission (CTRL-D)
			Key::Ctrl('d') => Err(ReadlineError::Eof)?,
			// End of text (CTRL-C)
			Key::Ctrl('c') => {
				self.print(&format!("{}{}", self.prompt, self.line), term)?;
				self.line.clear();
				self.cursor_pos = 0;
				self.clear_and_render(term)?;
				Err(ReadlineError::Interrupted)?
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
			}
			_ => {}
		}
		Ok(None)
	}
}

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

/// Structure that contains all the data necessary to read and write lines in a asyncronous manner
pub struct Readline<R: AsyncRead + Unpin> {
	raw_term: RawTerminal<Stdout>,
	event_stream: EventStream<R>, // Stream of events
	line_receiver: Receiver<Vec<u8>>,

	line: LineState, // Current line
}

impl<R: AsyncRead + Unpin> Readline<R> {
	pub fn new(prompt: String, reader: R) -> Result<(Self, SharedWriter), ReadlineError> {
		let (sender, line_receiver) = thingbuf::mpsc::channel(100);
		let mut readline = Readline {
			raw_term: stdout().into_raw_mode()?,
			event_stream: event_stream(reader),
			line_receiver,
			line: LineState::new(prompt), // Current line state
		};
		readline.line.render(&mut readline.raw_term)?;
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
				},
				result = self.line_receiver.recv_ref().fuse() => match result {
					Some(buf) => {
						self.line.print_data(&buf, &mut self.raw_term)?;
						return None
					},
					None => Err(ReadlineError::Closed)?,
				}
			}
		};
		Some(res)
	}
}
