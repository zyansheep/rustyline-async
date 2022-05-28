use std::{
	io::{self, stdout, Stdout, Write},
	ops::DerefMut,
	pin::Pin,
	task::{Context, Poll},
};

use crossterm::{
	event::EventStream,
	QueueableCommand,
	terminal::{self, disable_raw_mode},
};
use futures::{channel::mpsc, prelude::*};
use thingbuf::mpsc::{Receiver, Sender, errors::TrySendError};
use thiserror::Error;

mod line;
mod history;
use line::LineState;
use history::History;

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

	history_sender: Option<mpsc::Sender<String>>,
}

impl Readline {
	pub fn new(prompt: String) -> Result<(Self, SharedWriter), ReadlineError> {
		Self::create(prompt, None)
	}
	pub fn with_history(prompt: String, history_max_size: usize) -> Result<(Self, SharedWriter), ReadlineError> {
		Self::create(prompt, Some(History::new(history_max_size)))
	}
	fn create(prompt: String, history: Option<(History, mpsc::Sender<String>)>) -> Result<(Self, SharedWriter), ReadlineError> {
		let (sender, line_receiver) = thingbuf::mpsc::channel(500);
		terminal::enable_raw_mode()?;

		let (history, history_sender) =  match history {
            Some((a, b)) => (Some(a), Some(b)),
            None => (None, None),
        };
		let mut readline = Readline {
			raw_term: stdout(),
			event_stream: EventStream::new(),
			line_receiver,
			line: LineState::new(prompt, terminal::size()?, history),
			history_sender
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
						match self.line.handle_event(event, &mut self.raw_term).await {
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
	pub async fn add_history_entry(&mut self, entry: String) -> Option<()> {
		if let Some(sender) = &mut self.history_sender {
			sender.send(entry).await.ok()
		} else { None }
	}
}

impl Drop for Readline {
	fn drop(&mut self) {
		let _ = disable_raw_mode();
	}
}
