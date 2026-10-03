//! Buffer live-frame bytes until the presentation boundary, not Canvas flushes.
//!
//! This is an application write batch, not an atomic terminal/resize protocol.
//! History stream switches and cursor queries retain explicit flush barriers.

use std::io::{self, Write};

pub(super) struct FrameWriter<'a> {
    inner: Box<dyn Write + Send + 'a>,
    bytes: Vec<u8>,
    staging: bool,
}

impl<'a> FrameWriter<'a> {
    pub(super) fn new(inner: Box<dyn Write + Send + 'a>) -> Self {
        Self {
            inner,
            bytes: Vec::new(),
            staging: false,
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
