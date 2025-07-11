// This example tests the functionality of stripping ANSI escape codes
// and reducing multi-byte characters to single character spaces in the
// prompt line.
//
// Testing should be done by running the example and checking if the
// prompt line is displayed correctly with color and that the cursor position
// is correct when the program first runs, when a line is entered,
// and when control-C is pressed.
//
// Note: This example requires unicode support in the terminal to render properly.

use rustyline_async::{Readline, ReadlineEvent};
use std::io::Write;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let (mut rl, mut stdout) = Readline::new("\x1b[1;31m🢡🢡🢡 \x1b[0m".into())?;

	rl.should_print_line_on(false, false);

	loop {
		tokio::select! {
			_ = sleep(Duration::from_secs(1)) => {
				writeln!(stdout, "Message received!")?;
			}
			cmd = rl.readline() => match cmd {
				Ok(ReadlineEvent::Line(line)) => {
					writeln!(stdout, "You entered: {line:?}")?;
					rl.add_history_entry(line.clone());
					if line == "quit" {
						break;
					}
				}
				Ok(ReadlineEvent::Eof) => {
					writeln!(stdout, "<EOF>")?;
					break;
				}
				Ok(ReadlineEvent::Interrupted) => {
					// writeln!(stdout, "^C")?;
					continue;
				}
				Err(e) => {
					writeln!(stdout, "Error: {e:?}")?;
					break;
				}
			}
		}
	}
	rl.flush()?;
	Ok(())
}
