// TODO: http://rachid.koucha.free.fr/tech_corner/pty_pdip.html

extern crate futures;
extern crate libc;
extern crate termios;

use std::{pin::Pin, task::{Context, Poll}};

use libc::STDIN_FILENO;

use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, FutureExt, Stream, io, lock::BiLock, ready};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AsyncRustyLineError {
	#[error("io: {0}")]
	IO(#[from] io::Error),
	#[error("utf: {0}")]
	Utf(#[from] std::str::Utf8Error),
}

pub trait IOImpl {
	type Reader: AsyncRead + Unpin;
	type Writer: AsyncWrite + Unpin;
}

pub struct Line {
	pub line: Vec<u8>,
	pub text_last_nl: bool,
}

struct ReadlineInner<C: IOImpl> {
	stdin: C::Reader,
	stdout: C::Writer,
	prompt: String,

	line: String,

	lines_ready: Vec<String>,

	text_last_nl: bool,

	pending: String,
}

pub struct Lines<C: IOImpl> {
	inner: BiLock<ReadlineInner<C>>,
}

pub struct Writer<C: IOImpl> {
	inner: BiLock<ReadlineInner<C>>,
}

impl<C: IOImpl> ReadlineInner<C> {
	fn clear_line(&mut self) {
		self.pending += "\x1b[2K";
		self.pending += "\x1b[1000D";
	}

	fn redraw_line(&mut self) {
		self.pending += "\x1b[2K";
		self.pending += "\x1b[1000D";
		self.pending += &self.prompt;
		self.pending += &self.line;
	}

	fn leave_prompt(&mut self) {
		self.clear_line();
		self.restore_original();
		if !self.text_last_nl {
			self.pending += "\x1b[1B";
			self.pending += "\x1b[1A";
		}
	}

	fn enter_prompt(&mut self) {
		self.save_original();
		if !self.text_last_nl {
			//write!(self.pending, "\x1b[1E")?;
			self.pending += "\n";
			self.clear_line();
		}
	}

	fn save_original(&mut self) {
		self.pending += "\x1b[s";
	}

	fn restore_original(&mut self) {
		self.pending += "\x1b[u";
	}

	async fn write_pending(&mut self) -> io::Result<()> {
		let res = self.stdout.write_all(self.pending.as_bytes()).await;
		self.pending.clear();
		res
	}

	fn handle_char(&mut self, ch: char) {
		match ch {
			// Return
			'\x0d' => self
				.lines_ready
				.push(std::mem::replace(&mut self.line, String::new())),
			// Delete
			'\x7F' => {
				let _ = self.line.pop();
			}
			// End of transmission
			'\x04' => {
				let _ = self.line.pop();
			}
			_ => self.line.push(ch),
		}
	}

	async fn next_command(&mut self) -> Result<String, AsyncRustyLineError> {
		let mut tmp_buf = [0u8; 16];

		loop {
			let _ = self.write_pending().await;

			if let Some(line) = self.lines_ready.pop() {
				self.clear_line();
				let _ = self.write_pending().await;
				return Ok(line)
			}
			// FIXME: 0 means EOF?
			let bytes_read = self.stdin.read(&mut tmp_buf).await?;

			let string = std::str::from_utf8(&tmp_buf[0..bytes_read])?;
			for ch in string.chars() {
				self.handle_char(ch)
			}

			self.redraw_line();
		}
	}

	fn poll_write(&mut self, buf: &[u8]) -> Poll<Result<usize, io::Error>> {
		if buf.len() > 0 {
			self.leave_prompt();
			self.text_last_nl = buf[buf.len() - 1] == 10;
			let new_string = std::str::from_utf8(buf).map_err(|e|io::Error::new(io::ErrorKind::Other, e))?;
			self.pending += new_string;
			self.enter_prompt();
			self.redraw_line();
		}
		Poll::Ready(Ok(buf.len()))
	}

	fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
		let fut = self.write_pending();
		futures::pin_mut!(fut);
		fut.poll_unpin(cx)
	}
}

impl<C: IOImpl> Stream for Lines<C> {
    type Item = Result<String, AsyncRustyLineError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut guard = ready!(self.inner.poll_lock(cx));
		let next_command_fut = guard.next_command();
		futures::pin_mut!(next_command_fut);
		let res = ready!(next_command_fut.poll_unpin(cx));
		Poll::Ready(Some(res))
    }
}
impl<C: IOImpl> AsyncWrite for Writer<C> {
	fn poll_write(
			self: Pin<&mut Self>,
			cx: &mut Context<'_>,
			buf: &[u8],
		) -> Poll<io::Result<usize>> {
			let mut guard = ready!(self.inner.poll_lock(cx));
			guard.poll_write(buf)
	}

	fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
		let mut guard = ready!(self.inner.poll_lock(cx));
		guard.poll_flush(cx)
	}

	fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
		Poll::Ready(Ok(()))
	}
}

pub fn init<C: IOImpl>(stdin: C::Reader, stdout: C::Writer, prompt: String) -> (Lines<C>, Writer<C>) {
	let _ = enable_raw_mode();
	let mut inner = ReadlineInner {
		stdin: stdin,
		stdout: stdout,
		prompt,
		line: String::with_capacity(256),
		text_last_nl: true,
		pending: String::with_capacity(256),
		lines_ready: vec![],
	};

	let _ = inner.enter_prompt();

	let (l1, l2) = BiLock::new(inner);

	let writer = Writer { inner: l1 };
	let lines = Lines { inner: l2 };
	(lines, writer)
}

#[cfg(test)]
mod tests {
	#[test]
	fn it_works() {}
}

/// Call this function to enable line editing if you are sure that the stdin you passed is a TTY
pub fn enable_raw_mode() -> io::Result<termios::Termios> {
	let mut orig_term = termios::Termios::from_fd(STDIN_FILENO)?;

	// use nix::errno::Errno::ENOTTY;
	use termios::{
		BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG, ISTRIP, IXON,
		/* OPOST, */ VMIN, VTIME,
	};
	/* if !self.stdin_isatty {
		Err(nix::Error::from_errno(ENOTTY))?;
	} */
	termios::tcgetattr(STDIN_FILENO, &mut orig_term)?;
	let mut raw = orig_term;
	// disable BREAK interrupt, CR to NL conversion on input,
	// input parity check, strip high bit (bit 8), output flow control
	raw.c_iflag &= !(BRKINT | ICRNL | INPCK | ISTRIP | IXON);
	// we don't want raw output, it turns newlines into straight linefeeds
	// raw.c_oflag = raw.c_oflag & !(OPOST); // disable all output processing
	raw.c_cflag |= CS8; // character-size mark (8 bits)
				// disable echoing, canonical mode, extended input processing and signals
	raw.c_lflag &= !(ECHO | ICANON | IEXTEN | ISIG);
	raw.c_cc[VMIN] = 1; // One character-at-a-time input
	raw.c_cc[VTIME] = 0; // with blocking read
	termios::tcsetattr(STDIN_FILENO, termios::TCSADRAIN, &raw)?;
	Ok(orig_term)
}