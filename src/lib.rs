use std::{
	io::{self, stdout, Stdout, Write},
	ops::DerefMut,
	pin::Pin,
	task::{Context, Poll},
};

use crossterm::{
	event::EventStream,
	terminal::{self, disable_raw_mode},
	QueueableCommand,
};
use futures::{channel::mpsc, prelude::*};
use thingbuf::mpsc::{errors::TrySendError, Receiver, Sender};
use thiserror::Error;

mod history;
mod line;
use history::History;
use line::LineState;

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
	event_stream: EventStream, // Stream of events
	line_receiver: Receiver<Vec<u8>>,

	line: LineState, // Current line

	history_sender: mpsc::UnboundedSender<String>,
}

impl Readline {
	/// Create new Readline
	pub fn new(prompt: String) -> Result<(Self, SharedWriter), ReadlineError> {
		let (sender, line_receiver) = thingbuf::mpsc::channel(500);
		terminal::enable_raw_mode()?;

		let line = LineState::new(prompt, terminal::size()?);
		let history_sender = line.history.sender.clone();

		let mut readline = Readline {
			raw_term: stdout(),
			event_stream: EventStream::new(),
			line_receiver,
			line,
			history_sender,
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
	/// Set max history length
	pub fn set_max_history(&mut self, max_size: usize) {
		self.line.history.max_size = max_size;
		self.line.history.entries.truncate(max_size);
	}

	/// Set whether the input line should remain on the screen after
	/// events.
	///
	/// If `enter` is true, then when the user presses "Enter", the prompt
	/// and the text they entered will remain on the screen, and the cursor
	/// will move to the next line.  If `enter` is false, the prompt &
	/// input will be erased instead.
	///
	/// `control_c` similarly controls the behavior for when the user
	/// presses Ctrl-C.
	///
	/// The default value for both settings is `true`.
	pub fn should_print_line_on(&mut self, enter: bool, control_c: bool) {
		self.line.should_print_line_on_enter = enter;
		self.line.should_print_line_on_control_c = control_c;
	}

	/// Flush all writers to terminal
	pub fn flush(&mut self) -> Result<(), ReadlineError> {
		while let Ok(buf) = self.line_receiver.try_recv_ref() {
			self.line.print_data(&buf, &mut self.raw_term)?;
			self.line.clear(&mut self.raw_term)?;
		}
		self.raw_term.flush()?;
		Ok(())
	}

	/// Polling function for readline, manages all input and output.
	pub async fn readline(&mut self) -> Result<String, ReadlineError> {
		loop {
			futures::select! {
				event = self.event_stream.next().fuse() => match event {
					Some(Ok(event)) => {
						match self.line.handle_event(event, &mut self.raw_term).await {
							Ok(Some(line)) => return Result::<_, ReadlineError>::Ok(line),
							Err(e) => {
								self.raw_term.flush()?;
								return Err(e)
							},
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
	/// Add history entry asyncronously
	pub fn add_history_entry(&mut self, entry: String) -> Option<()> {
		self.history_sender.unbounded_send(entry).ok()
	}
}

impl Drop for Readline {
	fn drop(&mut self) {
		let _ = disable_raw_mode();
	}
}
