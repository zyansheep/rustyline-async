use rustyline_async::{Readline, ReadlineEvent};
use std::io::Write;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut rl, mut stdout) = Readline::new("prompt> ".into())?;

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
