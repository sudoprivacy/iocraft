//! Resize control: after the first frame, no application bytes are written.
use crossterm::{cursor, event, terminal, ExecutableCommand};
use std::io::{self, Write};

fn main() -> io::Result<()> {
    let acknowledgment_path = std::env::var_os("IOCRAFT_STATIC_ACK")
        .expect("set IOCRAFT_STATIC_ACK to a fresh diagnostic file");
    let mut acknowledgment = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(acknowledgment_path)?;
    println!("ShellHistoryBeforeScode");
    for index in 0..70 {
        println!("Earlier history line {index}");
    }
    println!("SharedPartialJoined\nSameBatch:end\nStdoutPrefix:StderrSuffix");
    terminal::enable_raw_mode()?;
    let result = (|| -> io::Result<()> {
        let mut output = io::stdout().lock();
        output.execute(cursor::Hide)?;
        let rows = [
            format!("StatusMarker phase=5 {}", "long status ".repeat(8)),
            "TodoMarker 1 todos (1 done, 0 open)".to_string(),
            "─".repeat(240),
            "❯ DraftSurvives".to_string(),
            "─".repeat(240),
            "FooterMarker static 240x40".to_string(),
        ];
        for (index, row) in rows.iter().enumerate() {
            if index > 0 {
                output.write_all(b"\r\n")?;
            }
            write!(output, "{row}\x1b[K\x1b[0m")?;
        }
        match std::env::var("IOCRAFT_STATIC_CURSOR")
            .as_deref()
            .unwrap_or("top")
        {
            "top" => {
                output.execute(cursor::MoveToPreviousLine(5))?;
            }
            "bottom" => {
                output.execute(cursor::MoveToColumn(0))?;
            }
            "blank" => output.write_all(b"\r\n")?,
            other => panic!("unknown cursor mode: {other}"),
        }
        output.flush()?;
        drop(output);
        let mut count = 0;
        loop {
            if let event::Event::Key(key) = event::read()? {
                if key.kind != event::KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    event::KeyCode::F(6) => {
                        count += 1;
                        writeln!(acknowledgment, "{count}")?;
                        acknowledgment.flush()?;
                    }
                    event::KeyCode::Esc => break,
                    _ => {}
                }
            }
        }
        Ok(())
    })();
    let _ = io::stdout().execute(cursor::Show);
    let _ = terminal::disable_raw_mode();
    result
}
