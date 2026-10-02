//! Presentation state shared by the tree and every history-output hook.
//!
//! This is not a transcript cache. Only the last live frame and the current
//! batch of output are retained. Backend cursor geometry remains in TerminalImpl.

use super::TerminalImpl;
use crate::{canvas::Canvas, element::Output};
use crossterm::{cursor, QueueableCommand};
use std::io;

pub(crate) enum HistoryMessage {
    Stdout(String),
    StdoutNoNewline(String),
    Stderr(String),
    StderrNoNewline(String),
}

impl HistoryMessage {
    fn into_parts(self) -> (Output, String, bool) {
        match self {
            Self::Stdout(text) => (Output::Stdout, text, true),
            Self::StdoutNoNewline(text) => (Output::Stdout, text, false),
            Self::Stderr(text) => (Output::Stderr, text, true),
            Self::StderrNoNewline(text) => (Output::Stderr, text, false),
        }
    }
}

#[derive(Default)]
pub(super) struct PresentationState {
    previous: Option<Canvas>,
    pending: Vec<HistoryMessage>,
    // This belongs to the terminal, not to the hook which last printed text.
    appended_newline: Option<u16>,
}

impl PresentationState {
    pub(super) fn enqueue(&mut self, messages: impl IntoIterator<Item = HistoryMessage>) {
        self.pending.extend(messages);
    }

    pub(super) fn begin_frame(&mut self, backend: &mut dyn TerminalImpl) -> io::Result<()> {
        if backend.begin_frame()? {
            self.previous = None;
        }
        Ok(())
    }

    pub(super) fn clear(&mut self, backend: &mut dyn TerminalImpl) -> io::Result<()> {
        if self.previous.is_some() {
            backend.clear_canvas()?;
            self.previous = None;
        }
        Ok(())
    }

    pub(super) fn present(
        &mut self,
        backend: &mut dyn TerminalImpl,
        output: Output,
        canvas: Canvas,
    ) -> io::Result<()> {
        self.commit_history(backend, output)?;
        if self.previous.as_ref() != Some(&canvas) {
            backend.write_canvas(self.previous.as_ref(), &canvas)?;
        }
        // Do not acknowledge a failed write by advancing the diff baseline.
        self.previous = Some(canvas);
        Ok(())
    }

    fn commit_history(&mut self, backend: &mut dyn TerminalImpl, output: Output) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        self.clear(backend)?;
        let mut unfinished = self.appended_newline.is_some();
        if let Some(column) = self.appended_newline.take() {
            backend
                .dest()
                .queue(cursor::MoveUp(1))?
                .queue(cursor::MoveToColumn(column))?;
        }
        // The erase must reach the terminal before output on the other stream.
        backend.flush_dest()?;
        let newline = if backend.is_raw_mode_enabled() {
            "\r\n"
        } else {
            "\n"
        };
        let mut last_stream = None;
        for message in self.pending.drain(..) {
            let (stream, text, append_newline) = message.into_parts();
            if let Some(previous) = last_stream.filter(|previous| *previous != stream) {
                flush_history_stream(backend, output, previous)?;
            }
            let writer = history_stream(backend, output, stream);
            writer.write_all(text.as_bytes())?;
            if append_newline {
                writer.write_all(newline.as_bytes())?;
                unfinished = false;
            } else if !text.is_empty() {
                unfinished = !text.ends_with('\n');
            }
            last_stream = Some(stream);
        }
        if let Some(stream) = last_stream {
            flush_history_stream(backend, output, stream)?;
        }
        if unfinished {
            self.appended_newline = backend.cursor_column().ok();
            // If the position query is unsupported, finish the line rather
            // than painting the live frame over unfinished history.
            backend.dest().write_all(newline.as_bytes())?;
        }
        Ok(())
    }
}

fn flush_history_stream(
    backend: &mut dyn TerminalImpl,
    output: Output,
    stream: Output,
) -> io::Result<()> {
    if output == stream {
        backend.flush_dest()
    } else {
        backend.alt().flush()
    }
}

fn history_stream(
    backend: &mut dyn TerminalImpl,
    output: Output,
    stream: Output,
) -> &mut dyn io::Write {
    if output == stream {
        backend.dest()
    } else {
        backend.alt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TerminalEvent;
    use futures::stream::{self, BoxStream, StreamExt};
    use std::{
        io::Write,
        mem,
        sync::{Arc, Mutex},
    };

    struct BufferedWriter {
        bytes: Vec<u8>,
        wire: Arc<Mutex<Vec<u8>>>,
        fail: bool,
    }

    impl Write for BufferedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fail {
                return Err(io::Error::other("injected write failure"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::other("injected flush failure"));
            }
            self.wire.lock().unwrap().append(&mut self.bytes);
            Ok(())
        }
    }

    struct Backend {
        dest: BufferedWriter,
        alt: BufferedWriter,
        clears: usize,
        paints: Vec<bool>,
        invalidate: bool,
        fail_paint: bool,
        column: Option<u16>,
    }

    impl Backend {
        fn new() -> Self {
            let wire = Arc::new(Mutex::new(Vec::new()));
            Self {
                dest: BufferedWriter {
                    bytes: Vec::new(),
                    wire: wire.clone(),
                    fail: false,
                },
                alt: BufferedWriter {
                    bytes: Vec::new(),
                    wire,
                    fail: false,
                },
                clears: 0,
                paints: Vec::new(),
                invalidate: false,
                fail_paint: false,
                column: Some(7),
            }
        }

        fn take_wire(&mut self) -> String {
            self.dest.flush().unwrap();
            self.alt.flush().unwrap();
            String::from_utf8(mem::take(&mut *self.dest.wire.lock().unwrap())).unwrap()
        }
    }

    impl Write for Backend {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.dest.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.dest.flush()
        }
    }

    impl TerminalImpl for Backend {
        fn is_raw_mode_enabled(&self) -> bool {
            true
        }
        fn cursor_column(&mut self) -> io::Result<u16> {
            self.column
                .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "no cursor query"))
        }
        fn begin_frame(&mut self) -> io::Result<bool> {
            Ok(mem::take(&mut self.invalidate))
        }
        fn clear_canvas(&mut self) -> io::Result<()> {
            self.clears += 1;
            Ok(())
        }
        fn write_canvas(&mut self, previous: Option<&Canvas>, _canvas: &Canvas) -> io::Result<()> {
            if self.fail_paint {
                return Err(io::Error::other("injected paint failure"));
            }
            self.paints.push(previous.is_some());
            Ok(())
        }
        fn event_stream(&mut self) -> io::Result<BoxStream<'static, io::Result<TerminalEvent>>> {
            Ok(stream::pending().boxed())
        }
        fn dest(&mut self) -> &mut dyn Write {
            &mut self.dest
        }
        fn alt(&mut self) -> &mut dyn Write {
            &mut self.alt
        }
    }

    #[test]
    fn one_owner_invalidates_clear_and_resize_and_preserves_unchanged_frames() {
        let mut state = PresentationState::default();
        let mut backend = Backend::new();
        for _ in 0..2 {
            state
                .present(&mut backend, Output::Stdout, Canvas::new(8, 1))
                .unwrap();
        }
        assert_eq!(backend.paints, [false], "unchanged frames do not paint");
        state.clear(&mut backend).unwrap();
        state.clear(&mut backend).unwrap();
        assert_eq!(backend.clears, 1);
        state
            .present(&mut backend, Output::Stdout, Canvas::new(8, 1))
            .unwrap();
        backend.invalidate = true;
        state.begin_frame(&mut backend).unwrap();
        state
            .present(&mut backend, Output::Stdout, Canvas::new(8, 1))
            .unwrap();
        state
            .present(&mut backend, Output::Stdout, Canvas::new(8, 2))
            .unwrap();
        assert_eq!(backend.paints, [false, false, false, true]);
    }

    #[test]
    fn history_batches_share_one_clear_and_keep_buffered_stream_order() {
        for output in [Output::Stdout, Output::Stderr] {
            let mut state = PresentationState::default();
            let mut backend = Backend::new();
            state
                .present(&mut backend, output, Canvas::new(8, 1))
                .unwrap();
            state.enqueue([HistoryMessage::StdoutNoNewline("one".into())]);
            state.enqueue([
                HistoryMessage::StderrNoNewline("two".into()),
                HistoryMessage::Stdout("three".into()),
            ]);
            state
                .present(&mut backend, output, Canvas::new(8, 1))
                .unwrap();
            assert_eq!(backend.take_wire(), "onetwothree\r\n");
            assert_eq!(backend.clears, 1);
            assert_eq!(
                backend.paints,
                [false, false],
                "history invalidates the diff baseline"
            );
        }
    }

    #[test]
    fn partial_history_continues_across_batches_and_empty_messages() {
        let mut state = PresentationState::default();
        let mut backend = Backend::new();
        state.enqueue([HistoryMessage::StdoutNoNewline("partial".into())]);
        state
            .present(&mut backend, Output::Stdout, Canvas::new(8, 1))
            .unwrap();
        assert_eq!(backend.take_wire(), "partial\r\n");
        state.enqueue([HistoryMessage::StderrNoNewline(String::new())]);
        state
            .present(&mut backend, Output::Stdout, Canvas::new(8, 1))
            .unwrap();
        assert_eq!(backend.take_wire(), "\x1b[1A\x1b[8G\r\n");
        state.enqueue([HistoryMessage::Stderr("-continued".into())]);
        state
            .present(&mut backend, Output::Stdout, Canvas::new(8, 1))
            .unwrap();
        assert_eq!(backend.take_wire(), "\x1b[1A\x1b[8G-continued\r\n");
        assert!(state.appended_newline.is_none());
    }

    #[test]
    fn missing_cursor_query_still_separates_history_from_live_frame() {
        let mut state = PresentationState::default();
        let mut backend = Backend::new();
        backend.column = None;
        state.enqueue([HistoryMessage::StdoutNoNewline("partial".into())]);
        state
            .present(&mut backend, Output::Stdout, Canvas::new(8, 1))
            .unwrap();
        assert_eq!(backend.take_wire(), "partial\r\n");
        assert!(state.appended_newline.is_none());
    }

    #[test]
    fn failed_output_or_paint_does_not_advance_the_frame() {
        for history in [false, true] {
            let mut state = PresentationState::default();
            let mut backend = Backend::new();
            if history {
                state.enqueue([HistoryMessage::Stderr("not committed".into())]);
                backend.alt.fail = true;
            } else {
                backend.fail_paint = true;
            }
            assert!(state
                .present(&mut backend, Output::Stdout, Canvas::new(8, 1))
                .is_err());
            assert!(state.previous.is_none());
            assert!(backend.paints.is_empty());
        }
    }

    #[test]
    fn failed_flush_stops_before_committing_history() {
        let mut state = PresentationState::default();
        let mut backend = Backend::new();
        state.enqueue([HistoryMessage::Stderr("not committed".into())]);
        backend.dest.fail = true;
        assert!(state
            .present(&mut backend, Output::Stdout, Canvas::new(8, 1))
            .is_err());
        assert!(state.previous.is_none());
        assert!(backend.alt.bytes.is_empty());
        assert!(backend.paints.is_empty());
    }

    #[test]
    fn failed_repaint_retains_the_last_successful_baseline() {
        let mut state = PresentationState::default();
        let mut backend = Backend::new();
        state
            .present(&mut backend, Output::Stdout, Canvas::new(8, 1))
            .unwrap();
        backend.fail_paint = true;
        assert!(state
            .present(&mut backend, Output::Stdout, Canvas::new(8, 2))
            .is_err());
        assert_eq!(state.previous.as_ref().unwrap().height(), 1);
    }
}
