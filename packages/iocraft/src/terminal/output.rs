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
}

impl<'a> FrameWriter<'a> {
    pub(super) fn new(inner: Box<dyn Write + Send + 'a>) -> Self {
        Self {
            inner,
            bytes: Vec::new(),
            staging: false,
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
            #[cfg(not(windows))]
            let native: Option<(u16, u16)> = None;
            let _ = writeln!(
                trace,
                "native_before={native:?} bytes={:?}",
                String::from_utf8_lossy(bytes)
            );
        }
    }
}
