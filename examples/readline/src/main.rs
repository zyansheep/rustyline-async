#![feature(try_blocks)]

use async_std::{
	io::stdin,
	stream, task,
};
use rustyline_async::{Readline, ReadlineError};

use std::{io::Write, time::Duration};

use futures::prelude::*;

#[async_std::main]
async fn main() -> Result<(), ReadlineError> {
	let mut periodic_timer1 = stream::interval(Duration::from_secs(2));
	let mut periodic_timer2 = stream::interval(Duration::from_secs(3));

	let (mut rl, mut stdout) = Readline::new("> ".to_owned(), stdin()).unwrap();

	simplelog::WriteLogger::init(
		log::LevelFilter::Debug,
		simplelog::Config::default(),
		stdout.clone(),
	)
	.unwrap();

	let join = task::spawn(async move {
		let ret: Result<(), ReadlineError> = try {
			loop {
				futures::select! {
					_ = periodic_timer1.next().fuse() => {
						writeln!(stdout, "First timer went off!")?;
					}
					_ = periodic_timer2.next().fuse() => {
						//write!(stdout_2, "Second timer went off!")?;
						log::info!("Second timer went off!");
					}
					command = rl.readline().fuse() => if let Some(command) = command {
						match command {
							Ok(line) => writeln!(stdout, "Received line: {}", line)?,
							Err(ReadlineError::Eof) =>{ writeln!(stdout, "Exiting...")?; break },
							Err(ReadlineError::Interrupted) => writeln!(stdout, "CTRL-C")?,
							Err(err) => {
								write!(stdout, "Received err: {:?}", err)?;
								break;
							},
						}
					}
				}
				rl.flush()?;
			}
		};
		ret
	});

	println!("Exited with: {:?}", join.await);
	Ok(())
}
