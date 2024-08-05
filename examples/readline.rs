use async_std::stream;
use rustyline_async::{Readline, ReadlineError, ReadlineEvent};

use std::{io::Write, time::Duration};

use futures_util::{select, FutureExt, StreamExt};

#[async_std::main]
async fn main() -> Result<(), ReadlineError> {
	let mut periodic_timer1 = stream::interval(Duration::from_secs(2));
	let mut periodic_timer2 = stream::interval(Duration::from_secs(3));

	let (mut rl, mut stdout) = Readline::new("> ".to_owned()).unwrap();
	// Options:
	// rl.should_print_line_on(false, false);
	// rl.set_max_history(10);

	simplelog::WriteLogger::init(
		log::LevelFilter::Debug,
		simplelog::Config::default(),
		stdout.clone(),
	)
	.unwrap();

	let mut running_first = true;
	let mut running_second = false;

	loop {
		select! {
			_ = periodic_timer1.next().fuse() => {
				if running_first { writeln!(stdout, "First timer went off!")?; }
			}
			_ = periodic_timer2.next().fuse() => {
				if running_second { log::info!("Second timer went off!"); }
			}
			command = rl.readline().fuse() => match command {
				Ok(ReadlineEvent::Line(line)) => {
					let line = line.trim();
					rl.add_history_entry(line.to_owned());
					match line {
						"start task" => {
							writeln!(stdout, "Starting the task...")?;
							running_first = true;
						},
						"stop task" => {
							writeln!(stdout, "Stopping the task...")?;
							running_first = false;
						}
						"start logging" => {
							log::info!("Starting the logger...");
							running_second = true
						},
						"stop logging" => {
							log::info!("Stopping the logger...");
							running_second = false
						},
						"start printouts" => {
							rl.should_print_line_on(true, true);
						},
						"stop printouts" => {
							rl.should_print_line_on(false, false);
						},
						"info" => {
							writeln!(stdout, r"
hello there
I use NixOS btw
its pretty cool
							")?;
						}
            "help" => {
              writeln!(stdout,
r"Commands:
start <task|logging|printouts>
stop <task|logging|printouts>
info
help")?;
            }
						_ => writeln!(stdout, "Command not found: \"{}\"", line)?,
					}
				},
				Ok(ReadlineEvent::Eof) => { writeln!(stdout, "Exiting...")?; break },
				Ok(ReadlineEvent::Interrupted) => writeln!(stdout, "^C")?,
				// Err(ReadlineError::Closed) => break, // Readline was closed via one way or another, cleanup other futures here and break out of the loop
				Err(err) => {
					writeln!(stdout, "Received err: {:?}", err)?;
					writeln!(stdout, "Exiting...")?;
					break
				},
			}
		}
	}

	// Flush all writers to stdout
	rl.flush()?;

	Ok(())
}
