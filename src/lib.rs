#![feature(const_mut_refs)]
#![feature(try_blocks)]

use std::io::{self, Stdout, Write, stdin, stdout};

use futures::prelude::*;

use input_codec::InputStream;
use sluice::pipe::{PipeReader, PipeWriter};

use termion::{clear, cursor, event::{Event, Key}, event, raw::{IntoRawMode, RawTerminal}};
use thiserror::Error;

mod input_codec;

use input_codec::TermReadAsync;

#[derive(Debug, Error)]
pub enum ReadlineAsyncError {
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
	fn print_data(&self, data: &[u8], term: &mut impl Write) -> Result<(), ReadlineAsyncError> {
		self.clear(term)?;
		term.write(data)?;
		if !data.ends_with(&['\n' as u8]) { writeln!(term)?; }
		self.clear(term)?;
		self.render(term)?;
		Ok(())
	}
	fn print(&self, string: &str, term: &mut impl Write) -> Result<(), ReadlineAsyncError> {
		self.print_data(string.as_bytes(), term)?;
		Ok(())
	}
	fn handle_key(&mut self, key: Key, term: &mut impl Write) -> Result<Option<String>, ReadlineAsyncError> {
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
				Err(ReadlineAsyncError::Eof)?
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

pub struct ReadlineAsync {
	// stdout_reader: PipeReader,
	raw_term: RawTerminal<Stdout>,
	event_stream: InputStream<Event>,

	line: LineState, // Current line
}

impl ReadlineAsync {
	pub fn new(prompt: String, reader: impl AsyncRead + Send + Unpin + 'static) -> Result<Self, ReadlineAsyncError> {
		//let (stdout_reader, write) = sluice::pipe::pipe();
		let readline = ReadlineAsync {
			//stdout_reader,
			raw_term: stdout().into_raw_mode()?,
			event_stream: reader.events_stream(),
			line: LineState::new(prompt), // Current line state
		};
		Ok(readline)
		
	}
	pub fn print(&mut self, string: &str) -> Result<(), ReadlineAsyncError> {
		self.line.print(string, &mut self.raw_term)
	}
	pub fn flush(&mut self) -> io::Result<()> {
		self.raw_term.flush()
	}
	pub async fn readline(&mut self) -> Option<Result<String, ReadlineAsyncError>> {
		// let out_buffer = [0u8; 1024]; // buffers data coming from external sources on its way to stdout
		let res: Result<String, ReadlineAsyncError> = try {
			match self.event_stream.next().await {
				Some(Ok(Event::Key(key))) => {
					match self.line.handle_key(key, &mut self.raw_term) {
						Ok(Some(line)) => Result::<_, ReadlineAsyncError>::Ok(line)?,
						Err(e) => Err(e)?,
						Ok(None) => return None,
					}
				}
				Some(Ok(_)) => return None,
				Some(Err(e)) => Err(e)?,
				None => return None,
			}
			/* futures::select! {
				result = self.stdout_reader.read(&mut out_buffer).fuse() => match result {
					Ok(bytes_read) => {
						self.line.print_data(&out_buffer[0..bytes_read], &mut self.raw_out)?;
						return None
					}
					Err(e) => Err(e)?,
				},
				result = self.event_stream.next().fuse() => match result {
					Some(Ok(Event::Key(key))) => {
						match self.line.handle_key(key, &mut self.raw_out) {
							Ok(Some(line)) => Result::<_, ReadlineAsyncError>::Ok(line)?,
							Err(e) => Err(e)?,
							Ok(None) => return None,
						}
					}
					Some(Ok(_)) => return None,
					Some(Err(e)) => Err(e)?,
					None => return None,
				},
			} */
		};
		Some(res)
	}
}
/* impl Drop for ReadlineAsync {
    fn drop(&mut self) {
        let _ = disable_raw_mode(self.orig_term);
    }
} */


/* /// Call this function to enable line editing if you are sure that the stdin you passed is a TTY
pub fn enable_raw_mode() -> io::Result<Termios> {
	let mut orig_term = Termios::from_fd(libc::STDIN_FILENO)?;

	// use nix::errno::Errno::ENOTTY;
	use termios::{
		BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG, ISTRIP, IXON,
		/* OPOST, */ VMIN, VTIME,
	};
	/* if !self.stdin_isatty {
		Err(nix::Error::from_errno(ENOTTY))?;
	} */
	termios::tcgetattr(libc::STDIN_FILENO, &mut orig_term)?;
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
	termios::tcsetattr(libc::STDIN_FILENO, termios::TCSADRAIN, &raw)?;
	Ok(orig_term)
}
pub fn disable_raw_mode(term: Termios) -> io::Result<()> {
	let ret = termios::tcsetattr(libc::STDIN_FILENO, termios::TCSADRAIN, &term);
	println!("Disbaled RAW Terminal");
	ret
} */