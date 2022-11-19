#![allow(dead_code)]

use std::io::Write;

use rustyline_async::{Readline, ReadlineError};

#[derive(Debug)]
struct BigStruct {
	bytes: Vec<u8>,
	name: String,
	number: usize,
}

#[async_std::main]
async fn main() -> Result<(), ReadlineError> {
	let (mut rl, mut stdout) = Readline::new("> ".to_owned()).unwrap();

	let thingy = BigStruct {
		bytes: vec![1; 20],
		name: "Baloney Shmalony".to_owned(),
		number: 60,
	};

	simplelog::WriteLogger::init(
		log::LevelFilter::Debug,
		simplelog::Config::default(),
		stdout.clone(),
	)
	.unwrap();

	loop {
		match rl.readline().await {
			Ok(_) => {
				writeln!(stdout, "{:?}", thingy)?;
				log::info!("{:?}", thingy);
			}
			Err(ReadlineError::Eof) => {
				writeln!(stdout, "Exiting...")?;
				break;
			}
			Err(ReadlineError::Interrupted) => writeln!(stdout, "^C")?,
			Err(err) => {
				writeln!(stdout, "Received err: {:?}", err)?;
				break;
			}
		}
	}
	rl.flush()?;

	Ok(())
}
