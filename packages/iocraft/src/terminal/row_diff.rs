//! Reuse rows that moved within a fully addressable inline canvas.
//! Insert/delete operations affect only live rows; finalized history is untouched.

use crate::canvas::Canvas;
use crossterm::{cursor, QueueableCommand};
use std::io::{self, Write};

pub(super) enum RowShift {
    Insert {
        first: usize,
        count: usize,
    },
    Scroll {
        first: usize,
        count: usize,
        retained: usize,
    },
}

impl RowShift {
    pub(super) fn between(previous: &Canvas, next: &Canvas, viewport: (u16, u16)) -> Option<Self> {
        if previous.width() != next.width()
            || previous.width() > usize::from(viewport.0)
            || previous.height().max(next.height()) + 1 >= usize::from(viewport.1)
        {
            return None;
        }
        let old_height = previous.height();
        let new_height = next.height();
        // Keep search work bounded independently of the source paragraph.
        const MAX_SHIFT: usize = 8;
        if new_height > old_height {
            let count = new_height - old_height;
            if count > MAX_SHIFT {
                return None;
            }
            let retained = (0..old_height)
                .rev()
                .take_while(|&row| previous.row_eq_at(next, row, row + count))
                .count();
            let shift = Self::Insert {
                first: old_height - retained,
                count,
            };
            return (shift.savings(previous, next) > 0).then_some(shift);
        }
        if new_height != old_height {
            return None;
        }
        let first = (0..old_height).find(|&row| !previous.row_eq(next, row))?;
        let mut best = None;
        let mut best_savings = 0;
        for count in 1..=MAX_SHIFT.min(old_height.saturating_sub(first + 1)) {
            let retained = (first..old_height - count)
                .take_while(|&row| previous.row_eq_at(next, row + count, row))
                .count();
            let shift = Self::Scroll {
                first,
                count,
                retained,
            };
            let savings = shift.savings(previous, next);
            if savings > best_savings {
                best_savings = savings;
                best = Some(shift);
            }
        }
        best
    }

    fn savings(&self, previous: &Canvas, next: &Canvas) -> usize {
        let (reused, cleared) = match *self {
            Self::Insert { first, count } => (first + count..next.height(), first..first + count),
            Self::Scroll {
                first,
                count,
                retained,
            } => (
                first..first + retained,
                first + retained..first + retained + count,
            ),
        };
        let saved: usize = reused
            .filter(|&row| !previous.row_eq(next, row))
            .map(|row| row_bytes(next, row))
            .sum();
        let added: usize = cleared
            .filter(|&row| previous.row_eq(next, row))
            .map(|row| row_bytes(next, row))
            .sum();
        // Covers eight extra newlines or the bounded cursor/line commands.
        // Count row bytes with the painter itself so styles and wide cells do
        // not turn a guessed cell-count saving into extra terminal traffic.
        saved.saturating_sub(added.saturating_add(32))
    }

    /// Returns the cursor's logical row after moving the retained cells.
    pub(super) fn apply(&self, writer: &mut dyn Write, old_height: usize) -> io::Result<usize> {
        let (first, count) = match *self {
            Self::Insert { first, count } | Self::Scroll { first, count, .. } => (first, count),
        };
        let mut current = old_height.saturating_sub(1);
        if matches!(self, Self::Insert { .. }) {
            // Extend at the tail before inserting. Otherwise a footer at the
            // viewport bottom would be discarded by the terminal's insert.
            for _ in 0..count {
                writer.write_all(b"\r\n")?;
            }
            current += count;
        }
        if current > first {
            writer.queue(cursor::MoveToPreviousLine((current - first) as u16))?;
        }
        match *self {
            Self::Insert { .. } => {
                write!(writer, "\x1b[{count}L")?;
                Ok(first)
            }
            Self::Scroll { retained, .. } => {
                write!(writer, "\x1b[{count}M")?;
                writer.queue(cursor::MoveToNextLine(retained as u16))?;
                write!(writer, "\x1b[{count}L")?;
                Ok(first + retained)
            }
        }
    }

    #[inline]
    pub(super) fn row_unchanged(&self, previous: &Canvas, next: &Canvas, row: usize) -> bool {
        match *self {
            Self::Insert { first, count } if row >= first => row >= first + count,
            Self::Scroll {
                first,
                count,
                retained,
            } if (first..first + retained + count).contains(&row) => row < first + retained,
            _ => previous.row_eq(next, row),
        }
    }
}

#[derive(Default)]
struct ByteCounter(usize);

impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn row_bytes(canvas: &Canvas, row: usize) -> usize {
    let mut counter = ByteCounter::default();
    canvas
        .write_ansi_row_without_newline(row, &mut counter)
        .expect("counting an ANSI row cannot fail");
    counter.0
}
