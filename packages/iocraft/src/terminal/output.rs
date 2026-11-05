//! Buffer live-frame bytes until the presentation boundary, not Canvas flushes.
//!
//! This is an application write batch, not an atomic terminal/resize protocol.
//! History stream switches and cursor queries retain explicit flush barriers.

use std::io::{self, Write};

pub(super) struct FrameWriter<'a> {
    inner: Box<dyn Write + Send + 'a>,
    bytes: Vec<u8>,
    staging: bool,
    trace: Option<std::fs::File>,
    #[cfg(windows)]
    _diagnostic_mode: Option<DiagnosticOutputMode>,
}

impl<'a> FrameWriter<'a> {
    pub(super) fn new(inner: Box<dyn Write + Send + 'a>) -> Self {
        Self {
            inner,
            bytes: Vec::new(),
            staging: false,
            #[cfg(windows)]
            _diagnostic_mode: DiagnosticOutputMode::new(),
            trace: std::env::var_os("IOCRAFT_FRAME_WRITE_TRACE").and_then(|path| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .ok()
            }),
        }
    }

    pub(super) fn begin(&mut self) {
        self.staging = true;
    }

    pub(super) fn finish(&mut self) -> io::Result<()> {
        self.staging = false;
        self.flush_segment()
    }

    pub(super) fn flush_segment(&mut self) -> io::Result<()> {
        trace_bytes(&mut self.trace, &self.bytes);
        let result = self.inner.write_all(&self.bytes);
        // A short write followed by an error must never be replayed on drop.
        self.bytes.clear();
        result?;
        self.inner.flush()
    }
}

#[cfg(windows)]
struct DiagnosticOutputMode {
    console: crossterm_winapi::ConsoleMode,
    original: u32,
}

#[cfg(windows)]
impl DiagnosticOutputMode {
    fn new() -> Option<Self> {
        let requested = std::env::var("IOCRAFT_DIAG_DELAYED_WRAP").ok()?;
        assert_eq!(requested, "1", "explicit diagnostic output mode required");
        let console = crossterm_winapi::ConsoleMode::from(
            crossterm_winapi::Handle::current_out_handle().expect("diagnostic console handle"),
        );
        let original = console.mode().expect("diagnostic original output mode");
        // DISABLE_NEWLINE_AUTO_RETURN: test the documented deferred-wrap mode
        // independently of application layout, Unicode text and assertions.
        console
            .set_mode(original | 0x0008)
            .expect("diagnostic deferred-wrap mode");
        Some(Self { console, original })
    }
}

#[cfg(windows)]
impl Drop for DiagnosticOutputMode {
    fn drop(&mut self) {
        let _ = self.console.set_mode(self.original);
    }
}

impl Write for FrameWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.staging {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        } else {
            trace_bytes(&mut self.trace, bytes);
            self.inner.write(bytes)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.staging {
            Ok(())
        } else {
            self.flush_segment()
        }
    }
}

fn trace_bytes(trace: &mut Option<std::fs::File>, bytes: &[u8]) {
    if let Some(trace) = trace {
        if !bytes.is_empty() {
            #[cfg(windows)]
            let native = crossterm::cursor::position().ok();
            #[cfg(windows)]
            let mode = crossterm_winapi::Handle::current_out_handle()
                .ok()
                .and_then(|handle| crossterm_winapi::ConsoleMode::from(handle).mode().ok());
            #[cfg(not(windows))]
            let native: Option<(u16, u16)> = None;
            #[cfg(not(windows))]
            let mode: Option<u32> = None;
            let _ = writeln!(
                trace,
                "native_before={native:?} output_mode={mode:?} bytes={:?}",
                String::from_utf8_lossy(bytes)
            );
        }
    }
}
