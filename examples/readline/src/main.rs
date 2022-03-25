use async_std::{io::stdin, stream};
use rustyline_async::{Readline, ReadlineError};

use std::time::Duration;

use futures::prelude::*;

#[async_std::main]
async fn main() -> Result<(), ReadlineError> {
	let mut periodic_timer1 = stream::interval(Duration::from_secs(2));
	let mut periodic_timer2 = stream::interval(Duration::from_secs(3));

	let mut rl = Readline::new("> ".to_owned(), stdin()).unwrap();

	/* struct AsyncWriteWrapper<W: AsyncWrite + Unpin>(W);
	impl<W: AsyncWrite + Unpin> std::io::Write for AsyncWriteWrapper<W> {
		fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
			task::block_on(self.0.write(buf))
		}

		fn flush(&mut self) -> io::Result<()> {
			task::block_on(self.0.flush())
		}
	}

	let stdout = AsyncWriteWrapper(writer);
	simplelog::WriteLogger::init(log::LevelFilter::Debug, simplelog::Config::default(), stdout).unwrap(); */
	
	loop {
		futures::select! {
			_ = periodic_timer1.next().fuse() => {
				rl.print("First timer went off!")?;
				// log::info!("First timer went off!");
			}
			_ = periodic_timer2.next().fuse() => {
				rl.print("Second timer went off!")?;
			}
			command = rl.readline().fuse() => if let Some(command) = command {
				match command {
					Ok(line) => rl.print(&format!("Received line: {:?}", line))?,
					Err(ReadlineError::Eof) => rl.print("Exiting...")?,
					Err(ReadlineError::Interrupted) => rl.print(&format!("CTRL-C"))?,
					Err(err) => {
						rl.print(&format!("Received err: {:?}", err))?;
						break;
					},
				}
			}
		}
		rl.flush()?;
	}
	Ok(())
	// println!("Exited with: {:?}", join.await);
}
