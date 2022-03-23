use async_std::{io, stream, task};

use std::time::Duration;

use futures::prelude::*;

#[async_std::main]
async fn main() {
	let mut periodic_timer1 = stream::interval(Duration::from_secs(2));
	let mut periodic_timer2 = stream::interval(Duration::from_secs(3));

	struct StandardIO;
	impl rustyline_async::IOImpl for StandardIO {
		type Reader = io::Stdin;
		type Writer = io::Stdout;
	}
	let (mut commands, out_writer) = rustyline_async::init::<StandardIO>(io::stdin(), io::stdout(), "> ".to_owned());

	struct AsyncWriteWrapper<W: AsyncWrite + Unpin>(W);
	impl<W: AsyncWrite + Unpin> std::io::Write for AsyncWriteWrapper<W> {
		fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
			task::block_on(self.0.write(buf))
		}

		fn flush(&mut self) -> io::Result<()> {
			task::block_on(self.0.flush())
		}
	}

	simplelog::WriteLogger::init(log::LevelFilter::Debug, simplelog::Config::default(), AsyncWriteWrapper(out_writer)).unwrap();
	
	loop {
		futures::select! {
			_ = periodic_timer1.next().fuse() => {
				log::info!("First timer went off!");
			}
			_ = periodic_timer2.next().fuse() => {
				log::debug!("Second timer went off!");
			}
			command = commands.next().fuse() => if let Some(command) = command {
				match command {
					Ok(line) => println!("Received line: {:?}", line),
					Err(err) => println!("Received err: {:?}", err),
				}
			}
		}
	}
}
