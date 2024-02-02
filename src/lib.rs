//! The `rustyline-async` library lets you read user input from the terminal
//! line by line while concurrently writing lines to the same terminal.
//!
//! Usage
//! =====
//!
//! - Call [`Readline::new()`] to create a [`Readline`] instance and associated
//!   [`SharedWriter`].
//!
//! - Call [`Readline::readline()`] (most likely in a loop) to receive a line
//!   of input from the terminal.  The user entering the line can edit their
//!   input using the key bindings listed under "Input Editing" below.
//!
//! - After receiving a line from the user, if you wish to add it to the
//!   history (so that the user can retrieve it while editing a later line),
//!   call [`Readline::add_history_entry()`].
//!
//! - Lines written to the associated `SharedWriter` while `readline()` is in
//!   progress will be output to the screen above the input line.
//!
//! - When done, call [`Readline::flush()`] to ensure that all lines written to
//!   the `SharedWriter` are output.
//!
//! Input Editing
//! =============
//!
//! While entering text, the user can edit and navigate through the current
//! input line with the following key bindings:
//!
//! - Left, Right: Move cursor left/right
//! - Up, Down: Scroll through input history
//! - Ctrl-W: Erase the input from the cursor to the previous whitespace
//! - Ctrl-U: Erase the input before the cursor
//! - Ctrl-L: Clear the screen
//! - Ctrl-Left / Ctrl-Right: Move to previous/next whitespace
//! - Home: Jump to the start of the line
//!     - When the "emacs" feature (on by default) is enabled, Ctrl-A has the
//!       same effect.
//! - End: Jump to the end of the line
//!     - When the "emacs" feature (on by default) is enabled, Ctrl-E has the
//!       same effect.
//! - Ctrl-D: Send an `Eof` event
//! - Ctrl-C: Send an `Interrupt` event

use std::{
	io::{self, stdout, Stdout, Write},
	ops::DerefMut,
	pin::Pin,
	task::{Context, Poll},
};

use crossterm::{
	event::EventStream,
	terminal::{self, disable_raw_mode, Clear},
	QueueableCommand,
};
use futures_channel::mpsc;
use futures_util::{pin_mut, ready, select, AsyncWrite, FutureExt, StreamExt};
use thingbuf::mpsc::{errors::TrySendError, Receiver, Sender};
use thiserror::Error;

mod history;
mod line;
use history::History;
use line::LineState;

/// Error returned from [`readline()`][Readline::readline].  Such errors
/// generally require specific procedures to recover from.
#[derive(Debug, Error)]
pub enum ReadlineError {
	/// An internal I/O error occurred
	#[error(transparent)]
	IO(#[from] io::Error),

	/// `readline()` was called after the [`SharedWriter`] was dropped and
	/// everything written to the `SharedWriter` was already output
	#[error("line writers closed")]
	Closed,
}

/// Events emitted by [`Readline::readline()`]
#[derive(Debug)]
pub enum ReadlineEvent {
	/// The user entered a line of text
	Line(String),
	/// The user pressed Ctrl-D
	Eof,
	/// The user pressed Ctrl-C
	Interrupted,
}

/// Clonable object that implements [`Write`][std::io::Write] and
/// [`AsyncWrite`][futures::io::AsyncWrite] and allows for sending data to the
/// terminal without messing up the readline.
///
/// A `SharedWriter` instance is obtained by calling [`Readline::new()`], which
/// also returns a [`Readline`] instance associated with the writer.
///
/// Data written to a `SharedWriter` is only output when a line feed (`'\n'`)
/// has been written and either [`Readline::readline()`] or
/// [`Readline::flush()`] is executing on the associated `Readline` instance.
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
			pin_mut!(fut);
			let mut send_buf = ready!(fut.poll_unpin(cx)).map_err(|_| {
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
		pin_mut!(fut);
		let mut send_buf = ready!(fut.poll_unpin(cx))
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

/// Structure for reading lines of input from a terminal while lines are output
/// to the terminal concurrently.
///
/// Terminal input is retrieved by calling [`Readline::readline()`], which
/// returns each complete line of input once the user presses Enter.
///
/// Each `Readline` instance is associated with one or more [`SharedWriter`]
/// instances.  Lines written to an associated `SharedWriter` are output while
/// retrieving input with `readline()` or by calling
/// [`flush()`][Readline::flush].
pub struct Readline {
	raw_term: Stdout,
	event_stream: EventStream, // Stream of events
	line_receiver: Receiver<Vec<u8>>,

	line: LineState, // Current line

	history_sender: mpsc::UnboundedSender<String>,
}

impl Readline {
	/// Create a new `Readline` instance with an associated
	/// [`SharedWriter`]
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

	/// Change the prompt
	pub fn update_prompt(&mut self, prompt: &str) -> Result<(), ReadlineError> {
		self.line.update_prompt(prompt, &mut self.raw_term)?;
		Ok(())
	}

	/// Clear the screen
	pub fn clear(&mut self) -> Result<(), ReadlineError> {
		self.raw_term.queue(Clear(terminal::ClearType::All))?;
		self.line.clear_and_render(&mut self.raw_term)?;
		self.raw_term.flush()?;
		Ok(())
	}

	/// Set maximum history length.  The default length is 1000.
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

	/// Flush all writers to terminal and erase the prompt string
	pub fn flush(&mut self) -> Result<(), ReadlineError> {
		while let Ok(buf) = self.line_receiver.try_recv_ref() {
			self.line.print_data(&buf, &mut self.raw_term)?;
		}
		self.line.clear(&mut self.raw_term)?;
		self.raw_term.flush()?;
		Ok(())
	}

	/// Polling function for readline, manages all input and output.
	/// Returns either an Readline Event or an Error
	pub async fn readline(&mut self) -> Result<ReadlineEvent, ReadlineError> {
		loop {
			select! {
				event = self.event_stream.next().fuse() => match event {
					Some(Ok(event)) => {
						match self.line.handle_event(event, &mut self.raw_term) {
							Ok(Some(event)) => {
								self.raw_term.flush()?;
								return Result::<_, ReadlineError>::Ok(event)
							},
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
				},
				_ = self.line.history.update().fuse() => {}
			}
		}
	}

	/// Add a line to the input history
	pub fn add_history_entry(&mut self, entry: String) -> Option<()> {
		self.history_sender.unbounded_send(entry).ok()
	}
}

impl Drop for Readline {
	fn drop(&mut self) {
		let _ = disable_raw_mode();
	}
}
