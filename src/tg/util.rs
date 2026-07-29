use color_eyre::Result;
use std::io::{self, BufRead, Write};
pub fn prompt(message: &str) -> Result<String> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(message.as_bytes())?;
    stdout.flush()?;

    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}
