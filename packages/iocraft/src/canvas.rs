use crate::style::{Color, Weight};
use crossterm::{
    csi,
    style::{Attribute, Colored},
};
use std::{
    env,
    fmt::{self, Display},
    io::{self, Write},
    ops::Range,
    sync::{Arc, Once, OnceLock},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Debug, PartialEq)]
struct Character {
    value: Arc<str>,
    style: Arc<CanvasTextStyle>,
}

static mut HANDLES_VS16_INCORRECTLY: bool = false;
static INIT_HANDLES_VS16_INCORRECTLY: Once = Once::new();

// Some terminals incorrectly only advance the cursor one space for emoji with VS16, so we need to
// add whitespace to compensate.
//
// https://www.jeffquast.com/post/ucs-detect-test-results/
// https://darrenburns.net/posts/emoji-in-the-terminal/
//
// Windows and iTerm2 seem to do the right thing. We add exceptions below for the ones that don't.
// Hopefully one day we'll be able to remove this hack.
pub(crate) fn handles_vs16_incorrectly() -> bool {
    unsafe {
        INIT_HANDLES_VS16_INCORRECTLY.call_once(|| {
            HANDLES_VS16_INCORRECTLY = env::var("TERM_PROGRAM")
                .map(|s| s == "Apple_Terminal")
                .unwrap_or(false)
                || env::var("GNOME_TERMINAL_SCREEN").is_ok_and(|v| !v.is_empty())
        });
        HANDLES_VS16_INCORRECTLY
    }
}

// Share bounded ASCII glyphs and keep each cell's ownership pointer compact.
// Multibyte glyphs retain their original UTF-8 bytes in owned shared storage.
static ASCII_CELLS: OnceLock<[Arc<str>; 128]> = OnceLock::new();

impl Character {
    fn new(value: &str, style: Arc<CanvasTextStyle>) -> Self {
        let value = if let [byte] = value.as_bytes() {
            // A valid single-byte UTF-8 string is always ASCII.
            let cells = ASCII_CELLS.get_or_init(|| {
                std::array::from_fn(|index| {
                    let byte = u8::try_from(index).expect("ASCII indices fit in u8");
                    Arc::from(char::from(byte).to_string())
                })
            });
            Arc::clone(&cells[usize::from(*byte)])
        } else {
            Arc::from(value)
        };
        Self { value, style }
    }

    fn required_padding(&self) -> usize {
        if self.value.contains('\u{fe0f}') {
            if handles_vs16_incorrectly() {
                self.value.width() - 1
            } else {
                0
            }
        } else {
            0
        }
    }
}

/// Describes the style of text to be rendered via a [`Canvas`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasTextStyle {
    /// The color of the text.
    pub color: Option<Color>,

    /// The weight of the text.
    pub weight: Weight,

    /// Whether the text is dim, independently of its weight. `Weight::Light`
    /// also enables dim text for backwards compatibility.
    pub dim: bool,

    /// Whether the text has a strikethrough decoration.
    pub strikethrough: bool,

    /// Whether the text is underlined.
    pub underline: bool,

    /// Whether the text is italicized.
    pub italic: bool,

    /// Whether the foreground and background colors should be inverted.
    pub invert: bool,
}

impl CanvasTextStyle {
    fn is_dim(self) -> bool {
        self.dim || self.weight == Weight::Light
    }
}

/// A single cell on a [`Canvas`], containing optional text and background color.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanvasCell {
    /// The background color of this cell, if set.
    pub background_color: Option<Color>,
    /// The OSC 8 hyperlink target for this cell, if set.
    pub hyperlink: Option<Arc<str>>,
    character: Option<Character>,
}

impl CanvasCell {
    /// Returns the text content of this cell, or `None` if empty.
    pub fn text(&self) -> Option<&str> {
        self.character.as_ref().map(|ch| ch.value.as_ref())
    }

    /// Returns the text style of this cell, or `None` if the cell is empty.
    pub fn text_style(&self) -> Option<&CanvasTextStyle> {
        self.character.as_ref().map(|ch| ch.style.as_ref())
    }

    /// Returns `true` if the cell has no content and no background color.
    pub fn is_empty(&self) -> bool {
        self.background_color.is_none() && self.character.is_none()
    }
}

/// `Canvas` is the medium that output is drawn to before being rendered to the terminal or other
/// destinations.
///
/// Typical use of the library doesn't require direct interaction with this struct. It is primarily useful for two cases:
///
/// - When implementing low-level components, you'll need to utilize the `Canvas` drawing methods.
/// - When implementing unit tests for components, you may want to render to a `Canvas` for inspection.
#[derive(Clone, PartialEq)]
pub struct Canvas {
    width: usize,
    cells: Vec<Vec<CanvasCell>>,
}

impl Canvas {
    /// Constructs a new canvas with the given dimensions.
    pub fn new(width: usize, height: usize) -> Self {
        let mut canvas = Self {
            width,
            cells: Vec::new(),
        };
        canvas.clear_and_resize(width, height);
        canvas
    }

    pub(crate) fn clear_and_resize(&mut self, width: usize, height: usize) {
        let is_narrower = width < self.width;
        self.width = width;
        self.cells.resize_with(height, Vec::new);
        for row in &mut self.cells {
            row.clear();
            if is_narrower {
                row.shrink_to(width);
            }
            row.resize_with(width, CanvasCell::default);
        }
    }

    /// Returns the width of the canvas.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Returns the height of the canvas.
    pub fn height(&self) -> usize {
        self.cells.len()
    }

    /// Returns a reference to the cell at the given position, or `None` if
    /// out of bounds.
    pub fn cell(&self, x: usize, y: usize) -> Option<&CanvasCell> {
        self.cells.get(y).and_then(|row| row.get(x))
    }

    /// Extracts plain text from a rectangular region of the canvas.
    ///
    /// Each row within the region produces one line in the result, separated
    /// by newlines. Trailing whitespace on each line is trimmed. Out-of-bounds
    /// coordinates are clamped silently.
    pub fn get_text(&self, x: usize, y: usize, w: usize, h: usize) -> String {
        let mut lines = Vec::with_capacity(h);
        for row_idx in y..y + h {
            let Some(row) = self.cells.get(row_idx) else {
                lines.push(String::new());
                continue;
            };
            let start = x.min(row.len());
            let end = (x + w).min(row.len());
            let slice = &row[start..end];
            let last_non_empty = slice.iter().rposition(|cell| cell.character.is_some());
            let trim_end = match last_non_empty {
                Some(i) => i + 1,
                None => {
                    lines.push(String::new());
                    continue;
                }
            };
            let mut s = String::with_capacity(trim_end);
            for cell in &slice[..trim_end] {
                match cell.character.as_ref() {
                    Some(ch) => s.push_str(&ch.value),
                    None => s.push(' '),
                }
            }
            lines.push(s);
        }
        lines.join(
            "
",
        )
    }

    fn clear_text(&mut self, x: usize, y: usize, w: usize, h: usize) {
        for y in y..y + h {
            if let Some(row) = self.cells.get_mut(y) {
                for x in x..x + w {
                    if x < row.len() {
                        row[x].character = None;
                    }
                }
            }
        }
    }

    fn set_background_color(&mut self, x: usize, y: usize, w: usize, h: usize, color: Color) {
        for y in y..y + h {
            if let Some(row) = self.cells.get_mut(y) {
                for x in x..x + w {
                    if x < row.len() {
                        row[x].background_color = Some(color);
                    }
                }
            }
        }
    }

    fn set_hyperlink(&mut self, x: usize, y: usize, w: usize, h: usize, url: Option<Arc<str>>) {
        for y in y..y + h {
            if let Some(row) = self.cells.get_mut(y) {
                for x in x..x + w {
                    if x < row.len() {
                        row[x].hyperlink = url.clone();
                    }
                }
            }
        }
    }

    fn set_text_row_chars<I>(&mut self, mut x: usize, y: usize, chars: I, style: CanvasTextStyle)
    where
        I: IntoIterator<Item = char>,
    {
        // Divide the string up into characters, which may consist of multiple Unicode code points.
        let row = &mut self.cells[y];
        let style = Arc::new(style);
        let mut buf = String::new();
        for c in chars.into_iter() {
            if x >= row.len() {
                break;
            }
            let width = c.width().unwrap_or(0);
            if width > 0 && !buf.is_empty() {
                row[x].character = Some(Character::new(&buf, Arc::clone(&style)));
                x += buf.width().max(1);
                buf.clear();
            }
            buf.push(c);
        }
        if !buf.is_empty() && x < row.len() {
            row[x].character = Some(Character::new(&buf, style));
        }
    }

    /// Gets a subview of the canvas for writing.
    pub fn subview_mut(
        &mut self,
        x: isize,
        y: isize,
        clip_x: isize,
        clip_y: isize,
        clip_width: usize,
        clip_height: usize,
    ) -> CanvasSubviewMut<'_> {
        CanvasSubviewMut {
            y,
            x,
            clip_x,
            clip_y,
            clip_width,
            clip_height,
            canvas: self,
        }
    }

    fn row(&self, y: usize) -> Option<&[CanvasCell]> {
        let row = self.cells.get(y)?;
        let last_non_empty = row.iter().rposition(|cell| !cell.is_empty());
        Some(&row[..last_non_empty.map_or(0, |i| i + 1)])
    }

    pub(crate) fn reflowed_height(&self, columns: usize) -> usize {
        self.reflowed_prefix_height(columns, self.height())
    }

    pub(crate) fn reflowed_prefix_height(&self, columns: usize, height: usize) -> usize {
        let columns = columns.max(1);
        (0..height.min(self.height()))
            .map(|y| {
                let row = self.row(y).unwrap_or(&[]);
                let (mut source, mut column, mut rows) = (0, 0, 1);
                while source < row.len() {
                    let width = row[source]
                        .character
                        .as_ref()
                        .map_or(1, |c| c.value.width().max(1));
                    if column > 0 && column + width > columns {
                        rows += 1;
                        column = 0;
                    }
                    column += width;
                    source += width;
                }
                rows
            })
            .sum()
    }

    pub(crate) fn row_eq(&self, other: &Self, y: usize) -> bool {
        self.row_eq_at(other, y, y)
    }

    #[inline]
    pub(crate) fn row_eq_at(&self, other: &Self, row: usize, other_row: usize) -> bool {
        self.width == other.width && self.row(row) == other.row(other_row)
    }

    /// Writes a single row.
    ///
    /// In ANSI mode the caller must ensure that SGR state is reset (e.g. via
    /// `CSI 0 m`) before invoking this method; the function does not emit a
    /// leading reset of its own. It always leaves SGR state reset on return,
    /// so consecutive calls (or any subsequent writer use) start from a clean
    /// state.
    fn write_row_impl<W: Write>(&self, y: usize, mut w: W, ansi: bool) -> io::Result<()> {
        let row = self.row(y).unwrap_or(&[]);
        self.write_row_range(y, 0..row.len(), &mut w, ansi, true)
    }

    /// Paint a range with independent SGR/link state. A partial range must not
    /// erase the unchanged suffix; a default-only tail can still use EL.
    fn write_row_range<W: Write>(
        &self,
        y: usize,
        range: Range<usize>,
        mut w: W,
        ansi: bool,
        clear_tail: bool,
    ) -> io::Result<()> {
        let row = self.cells.get(y).map(Vec::as_slice).unwrap_or(&[]);

        let mut background_color = None;
        let mut hyperlink: Option<Arc<str>> = None;
        let mut text_style = CanvasTextStyle::default();
        let mut col = range.start;
        let mut did_clear_line = false;
        while col < range.end {
            let cell = &row[col];

            if ansi {
                let mut needs_reset = false;
                if let Some(c) = &cell.character {
                    // SGR bold and dim are independent. Enabling one does not
                    // turn the other off, so reset when either is removed.
                    if (text_style.weight == Weight::Bold && c.style.weight != Weight::Bold)
                        || (text_style.is_dim() && !c.style.is_dim())
                    {
                        needs_reset = true;
                    }
                    if !c.style.strikethrough && text_style.strikethrough {
                        needs_reset = true;
                    }
                    if !c.style.underline && text_style.underline {
                        needs_reset = true;
                    }
                    if !c.style.italic && text_style.italic {
                        needs_reset = true;
                    }
                    if !c.style.invert && text_style.invert {
                        needs_reset = true;
                    }
                } else if text_style.underline || text_style.invert || text_style.strikethrough {
                    needs_reset = true;
                }
                if needs_reset {
                    write!(w, csi!("0m"))?;
                    background_color = None;
                    text_style = CanvasTextStyle::default();
                }

                if let Some(c) = &cell.character {
                    if c.style.color != text_style.color {
                        write!(
                            w,
                            csi!("{}m"),
                            Colored::ForegroundColor(c.style.color.unwrap_or(Color::Reset))
                        )?;
                    }

                    if c.style.weight == Weight::Bold && text_style.weight != Weight::Bold {
                        write!(w, csi!("{}m"), Attribute::Bold.sgr())?;
                    }

                    if c.style.is_dim() && !text_style.is_dim() {
                        write!(w, csi!("{}m"), Attribute::Dim.sgr())?;
                    }

                    if c.style.strikethrough && !text_style.strikethrough {
                        write!(w, csi!("{}m"), Attribute::CrossedOut.sgr())?;
                    }

                    if c.style.underline && !text_style.underline {
                        write!(w, csi!("{}m"), Attribute::Underlined.sgr())?;
                    }

                    if c.style.italic && !text_style.italic {
                        write!(w, csi!("{}m"), Attribute::Italic.sgr())?;
                    }

                    if c.style.invert && !text_style.invert {
                        write!(w, csi!("{}m"), Attribute::Reverse.sgr())?;
                    }

                    text_style = *c.style;
                }
            }

            if let Some(c) = &cell.character {
                col += c.value.width().max(1);
            } else {
                col += 1;
            }

            if ansi && clear_tail && col >= self.width {
                if hyperlink.is_some() {
                    write!(w, "\x1b]8;;\x1b\\")?; // close any open OSC 8 link
                    hyperlink = None;
                }
                if background_color.is_some() {
                    write!(w, csi!("{}m"), Colored::BackgroundColor(Color::Reset))?;
                    background_color = None;
                }

                write!(w, csi!("K"))?;
                did_clear_line = true;
            }

            if ansi && cell.background_color != background_color {
                write!(
                    w,
                    csi!("{}m"),
                    Colored::BackgroundColor(cell.background_color.unwrap_or(Color::Reset))
                )?;
                background_color = cell.background_color;
            }

            if ansi && cell.hyperlink != hyperlink {
                // OSC 8: close the previous link, then open the new one (if any).
                match &cell.hyperlink {
                    Some(url) => write!(w, "\x1b]8;;{}\x1b\\", url)?,
                    None => write!(w, "\x1b]8;;\x1b\\")?,
                }
                hyperlink = cell.hyperlink.clone();
            }

            if let Some(c) = &cell.character {
                write!(w, "{}{}", c.value, " ".repeat(c.required_padding()))?;
            } else {
                w.write_all(b" ")?;
            }
        }
        if ansi {
            if hyperlink.is_some() {
                write!(w, "\x1b]8;;\x1b\\")?; // close any open OSC 8 link
            }
            if background_color.is_some() {
                write!(w, csi!("{}m"), Colored::BackgroundColor(Color::Reset))?;
            }
            if clear_tail && !did_clear_line {
                write!(w, csi!("K"))?;
            }
            write!(w, csi!("0m"))?;
        }
        Ok(())
    }

    /// Writes a single row's ANSI representation without a trailing newline.
    ///
    /// The caller must ensure SGR state is reset before this is called (the
    /// terminal's default state qualifies). The function leaves SGR state
    /// reset on return, so a sequence of calls — separated only by cursor
    /// movement — will each start from a clean state.
    pub(crate) fn write_ansi_row_without_newline<W: Write>(
        &self,
        y: usize,
        w: W,
    ) -> io::Result<()> {
        self.write_row_impl(y, w, true)
    }

    /// Update only changed cells when both canvases own the same row geometry.
    /// Glyph boundaries include the old glyph too, so replacing/removing a wide
    /// character never leaves its other cell behind or starts inside it.
    pub(crate) fn write_ansi_row_diff<W: Write>(
        &self,
        previous: &Self,
        y: usize,
        mut w: W,
    ) -> io::Result<()> {
        if self.width != previous.width || previous.cells.get(y).is_none() {
            return self.write_ansi_row_without_newline(y, w);
        }
        let Some(current) = self.cells.get(y) else {
            return Ok(());
        };
        let old = &previous.cells[y];
        let Some(start) = current.iter().zip(old).position(|(a, b)| a != b) else {
            return Ok(());
        };
        let end = current
            .iter()
            .zip(old)
            .rposition(|(a, b)| a != b)
            .expect("changed cell")
            + 1;
        let mut range = start..end;
        loop {
            let before = range.clone();
            for row in [old, current] {
                let mut column = 0;
                while column < range.end {
                    let width = row[column]
                        .character
                        .as_ref()
                        .map_or(1, |ch| ch.value.width().max(1));
                    if column < range.start && range.start < column + width {
                        range.start = column;
                    }
                    if column < range.end && range.end < column + width {
                        range.end = (column + width).min(self.width);
                    }
                    column += width;
                }
            }
            if before == range {
                break;
            }
        }
        if range.start > 0 {
            let column = u16::try_from(range.start).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "canvas column exceeds terminal geometry",
                )
            })?;
            use crossterm::QueueableCommand;
            w.queue(crossterm::cursor::MoveToColumn(column))?;
        }
        let content_end = self.row(y).map_or(0, <[CanvasCell]>::len);
        let clear_tail = range.end >= content_end;
        if clear_tail {
            range.end = content_end.max(range.start);
        }
        self.write_row_range(y, range, w, true, clear_tail)
    }

    fn write_impl<W: Write>(
        &self,
        mut w: W,
        ansi: bool,
        omit_final_newline: bool,
    ) -> io::Result<()> {
        if ansi {
            // Seed clean SGR state for the first row. Subsequent rows rely on
            // the trailing reset of the previous row.
            write!(w, csi!("0m"))?;
        }
        for y in 0..self.cells.len() {
            self.write_row_impl(y, &mut w, ansi)?;
            let is_final_line = y == self.cells.len() - 1;
            if !omit_final_newline || !is_final_line {
                if ansi {
                    // add a carriage return in case we're in raw mode
                    w.write_all(b"\r\n")?;
                } else {
                    w.write_all(b"\n")?;
                }
            }
        }
        w.flush()?;
        Ok(())
    }

    /// Writes the canvas to the given writer with ANSI escape codes.
    pub fn write_ansi<W: Write>(&self, w: W) -> io::Result<()> {
        self.write_impl(w, true, false)
    }

    pub(crate) fn write_ansi_without_final_newline<W: Write>(&self, w: W) -> io::Result<()> {
        self.write_impl(w, true, true)
    }

    /// Writes the canvas to the given writer as unstyled text, without ANSI escape codes.
    pub fn write<W: Write>(&self, w: W) -> io::Result<()> {
        self.write_impl(w, false, false)
    }
}

impl Display for Canvas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = Vec::with_capacity(self.width * self.cells.len());
        self.write(&mut buf).unwrap();
        f.write_str(&String::from_utf8_lossy(&buf))?;
        Ok(())
    }
}

/// Represents a writeable region of a [`Canvas`]. All coordinates provided to functions of this
/// type are relative to the region's top-left corner.
pub struct CanvasSubviewMut<'a> {
    x: isize,
    y: isize,
    clip_x: isize,
    clip_y: isize,
    clip_width: usize,
    clip_height: usize,
    canvas: &'a mut Canvas,
}

impl CanvasSubviewMut<'_> {
    /// Returns a reference to a cell at the given **relative** subview position.
    ///
    /// Returns `None` if the resulting absolute position is out of bounds or
    /// outside the clip region.
    pub fn cell(&self, x: isize, y: isize) -> Option<&CanvasCell> {
        let abs_x = self.x + x;
        let abs_y = self.y + y;
        if abs_x < self.clip_x
            || abs_y < self.clip_y
            || abs_x < 0
            || abs_y < 0
            || abs_x >= self.clip_x + self.clip_width as isize
            || abs_y >= self.clip_y + self.clip_height as isize
        {
            return None;
        }
        self.canvas.cell(abs_x as usize, abs_y as usize)
    }

    /// Extracts plain text from a rectangular region using **relative** subview
    /// coordinates. The region is clamped to the clip bounds.
    pub fn get_text(&self, x: isize, y: isize, w: usize, h: usize) -> String {
        let mut left = self.x + x;
        let mut top = self.y + y;
        let mut right = left + w as isize;
        let mut bottom = top + h as isize;

        left = left.max(self.clip_x).max(0);
        top = top.max(self.clip_y).max(0);
        right = right.min(self.clip_x + self.clip_width as isize).max(0);
        bottom = bottom.min(self.clip_y + self.clip_height as isize).max(0);

        self.canvas.get_text(
            left as _,
            top as _,
            (right - left).max(0) as _,
            (bottom - top).max(0) as _,
        )
    }

    /// Fills the region with the given color.
    pub fn set_background_color(&mut self, x: isize, y: isize, w: usize, h: usize, color: Color) {
        let mut left = self.x + x;
        let mut top = self.y + y;
        let mut right = left + w as isize;
        let mut bottom = top + h as isize;

        left = left.max(self.clip_x).max(0);
        top = top.max(self.clip_y).max(0);
        right = right.min(self.clip_x + self.clip_width as isize).max(0);
        bottom = bottom.min(self.clip_y + self.clip_height as isize).max(0);

        self.canvas.set_background_color(
            left as _,
            top as _,
            (right - left).max(0) as _,
            (bottom - top).max(0) as _,
            color,
        );
    }

    /// Sets the OSC 8 hyperlink for a rectangular region.
    pub fn set_hyperlink(&mut self, x: isize, y: isize, w: usize, h: usize, url: Option<Arc<str>>) {
        let mut left = self.x + x;
        let mut top = self.y + y;
        let mut right = left + w as isize;
        let mut bottom = top + h as isize;

        left = left.max(self.clip_x).max(0);
        top = top.max(self.clip_y).max(0);
        right = right.min(self.clip_x + self.clip_width as isize).max(0);
        bottom = bottom.min(self.clip_y + self.clip_height as isize).max(0);

        self.canvas.set_hyperlink(
            left as _,
            top as _,
            (right - left).max(0) as _,
            (bottom - top).max(0) as _,
            url,
        );
    }

    /// Removes text from the region.
    pub fn clear_text(&mut self, x: isize, y: isize, w: usize, h: usize) {
        let mut left = self.x + x;
        let mut top = self.y + y;
        let mut right = left + w as isize;
        let mut bottom = top + h as isize;

        left = left.max(self.clip_x).max(0);
        top = top.max(self.clip_y).max(0);
        right = right.min(self.clip_x + self.clip_width as isize).max(0);
        bottom = bottom.min(self.clip_y + self.clip_height as isize).max(0);

        self.canvas.clear_text(
            left as _,
            top as _,
            (right - left).max(0) as _,
            (bottom - top).max(0) as _,
        );
    }

    /// Writes text to the region.
    pub fn set_text(&mut self, x: isize, y: isize, text: &str, style: CanvasTextStyle) {
        let mut x = self.x + x;
        let min_x = self.clip_x.max(0);
        let mut to_skip = 0;
        if x < min_x {
            to_skip = min_x - x;
            x = min_x;
        }
        let max_x = self.clip_x + self.clip_width as isize - 1;
        let horizontal_space = max_x - x + 1;
        let min_y = self.clip_y.max(0);
        let max_y = (self.clip_y + self.clip_height as isize).min(self.canvas.height() as _) - 1;
        for (y, line) in (self.y + y..).zip(text.lines()) {
            if y >= min_y && y <= max_y {
                let mut skipped_width = 0;
                let mut taken_width = 0;
                self.canvas.set_text_row_chars(
                    x as usize,
                    y as usize,
                    line.chars()
                        .skip_while(|c| {
                            if skipped_width < to_skip {
                                skipped_width += c.width().unwrap_or(0) as isize;
                                true
                            } else {
                                false
                            }
                        })
                        .take_while(|c| {
                            if taken_width < horizontal_space {
                                taken_width += c.width().unwrap_or(0) as isize;
                                true
                            } else {
                                false
                            }
                        }),
                    style,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    fn prompt_canvas(text: &str, width: usize) -> super::Canvas {
        let mut canvas = super::Canvas::new(width, 3);
        let mut view = canvas.subview_mut(0, 0, 0, 0, width, 3);
        view.set_background_color(0, 0, width, 1, crate::Color::Blue);
        view.set_text(
            0,
            0,
            text,
            super::CanvasTextStyle {
                color: Some(crate::Color::White),
                ..Default::default()
            },
        );
        view.set_text(0, 1, "Unchanged suffix", super::CanvasTextStyle::default());
        view.set_text(0, 2, "FOOTER", super::CanvasTextStyle::default());
        canvas
    }

    #[test]
    fn changed_cells_match_a_fresh_paint_without_touching_the_suffix() {
        for (old, new) in [
            ("❯ draft", "❯ drafts"),
            ("❯ drafts", "❯ draft"),
            ("❯ 你🙂e\u{301} tail", "❯ x🙂e\u{301} tail"),
            ("❯ x🙂e\u{301} tail", "❯ 你🙂e\u{301} tail"),
            ("❯ 你 wide", "❯ "),
            ("❯ good", ""),
        ] {
            let previous = prompt_canvas(old, 80);
            let next = prompt_canvas(new, 80);
            let mut initial = Vec::new();
            previous
                .write_ansi_without_final_newline(&mut initial)
                .unwrap();
            let mut diff = Vec::new();
            next.write_ansi_row_diff(&previous, 0, &mut diff).unwrap();
            assert!(!String::from_utf8_lossy(&diff).contains("FOOTER"));
            let mut observed = avt::Vt::new(80, 5);
            observed.feed_str(&String::from_utf8(initial).unwrap());
            observed.feed_str("\x1b[1;1H");
            observed.feed_str(&String::from_utf8(diff).unwrap());
            let mut full = Vec::new();
            next.write_ansi_without_final_newline(&mut full).unwrap();
            let mut expected = avt::Vt::new(80, 5);
            expected.feed_str(&String::from_utf8(full).unwrap());
            for row in 0..3 {
                assert_eq!(
                    observed.line(row).text(),
                    expected.line(row).text(),
                    "{old:?} -> {new:?}"
                );
                for column in 0..80 {
                    let actual = &observed.line(row)[column];
                    let reference = &expected.line(row)[column];
                    assert_eq!(
                        actual.width(),
                        reference.width(),
                        "glyph boundary at {column}"
                    );
                    assert_eq!(
                        actual.pen().background(),
                        reference.pen().background(),
                        "background at {column}"
                    );
                    if next.cell(column, row).unwrap().text().is_some() {
                        assert_eq!(actual.pen(), reference.pen(), "text style at {column}");
                    } else {
                        // Empty canvas cells specify background, not the
                        // invisible foreground inherited by a printed space.
                        assert_eq!(actual.pen().is_underline(), reference.pen().is_underline());
                        assert_eq!(actual.pen().is_inverse(), reference.pen().is_inverse());
                    }
                }
            }
        }
    }

    #[test]
    fn a_shaded_prompt_edit_does_not_replay_its_unchanged_padding() {
        let previous = prompt_canvas("❯ draft", 200);
        let next = prompt_canvas("❯ drafts", 200);
        let mut diff = Vec::new();
        next.write_ansi_row_diff(&previous, 0, &mut diff).unwrap();
        assert!(
            diff.len() < 64,
            "one edit must not replay 200 colored cells: {diff:?}"
        );
        let mut same = Vec::new();
        next.write_ansi_row_diff(&next, 0, &mut same).unwrap();
        assert!(same.is_empty());
    }

    #[test]
    fn shrinking_plain_text_clears_only_the_default_tail() {
        let mut previous = super::Canvas::new(40, 2);
        previous.subview_mut(0, 0, 0, 0, 40, 2).set_text(
            0,
            0,
            "prefix and an old tail",
            super::CanvasTextStyle::default(),
        );
        let mut next = super::Canvas::new(40, 2);
        next.subview_mut(0, 0, 0, 0, 40, 2).set_text(
            0,
            0,
            "prefix",
            super::CanvasTextStyle::default(),
        );
        let mut initial = Vec::new();
        previous
            .write_ansi_without_final_newline(&mut initial)
            .unwrap();
        let mut diff = Vec::new();
        next.write_ansi_row_diff(&previous, 0, &mut diff).unwrap();
        assert!(diff.len() < 20);
        let mut vt = avt::Vt::new(40, 4);
        vt.feed_str(&String::from_utf8(initial).unwrap());
        vt.feed_str("\x1b[1;1H");
        vt.feed_str(&String::from_utf8(diff).unwrap());
        assert_eq!(vt.line(0).text().trim_end(), "prefix");
    }

    #[test]
    fn partial_link_updates_close_their_own_hyperlink_scope() {
        let previous = prompt_canvas("prefix linked suffix", 80);
        let mut next = prompt_canvas("prefix linked suffix", 80);
        next.subview_mut(0, 0, 0, 0, 80, 3).set_hyperlink(
            7,
            0,
            6,
            1,
            Some(std::sync::Arc::from("https://example.com/new;target")),
        );
        let mut diff = Vec::new();
        next.write_ansi_row_diff(&previous, 0, &mut diff).unwrap();
        let wire = String::from_utf8(diff).unwrap();
        assert!(wire.contains("\x1b]8;;https://example.com/new;target\x1b\\linked"));
        assert!(wire.contains("\x1b]8;;\x1b\\"));
        assert!(!wire.contains("suffix"));
    }

    #[test]
    fn reused_canvas_drops_old_text_background_and_links_on_resize() {
        let mut canvas = super::Canvas::new(8, 2);
        {
            let mut view = canvas.subview_mut(0, 0, 0, 0, 8, 2);
            view.set_text(0, 0, "old text", super::CanvasTextStyle::default());
            view.set_background_color(0, 0, 8, 2, crate::Color::Blue);
            view.set_hyperlink(
                0,
                0,
                8,
                2,
                Some(std::sync::Arc::from("https://example.com")),
            );
        }
        for (width, height) in [(4, 1), (8, 2)] {
            canvas.clear_and_resize(width, height);
            assert_eq!((canvas.width(), canvas.height()), (width, height));
            for y in 0..height {
                for x in 0..width {
                    let cell = canvas.cell(x, y).unwrap();
                    assert!(cell.is_empty());
                    assert!(cell.hyperlink.is_none());
                }
            }
        }
    }

    use super::*;
    use crate::prelude::*;

    #[test]
    fn test_canvas_background_color() {
        let mut canvas = Canvas::new(6, 3);
        assert_eq!(canvas.width(), 6);
        assert_eq!(canvas.height(), 3);

        canvas
            .subview_mut(2, 0, 2, 0, 3, 2)
            .set_background_color(0, 0, 5, 5, Color::Red);

        let mut actual = Vec::new();
        canvas.write_ansi(&mut actual).unwrap();

        let mut expected = Vec::new();
        // row 0
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "  ").unwrap();
        write!(expected, csi!("{}m"), Colored::BackgroundColor(Color::Red)).unwrap();
        write!(expected, "   ").unwrap();
        write!(
            expected,
            csi!("{}m"),
            Colored::BackgroundColor(Color::Reset)
        )
        .unwrap();
        write!(expected, csi!("K")).unwrap();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "\r\n").unwrap();
        // row 1
        write!(expected, "  ").unwrap();
        write!(expected, csi!("{}m"), Colored::BackgroundColor(Color::Red)).unwrap();
        write!(expected, "   ").unwrap();
        write!(
            expected,
            csi!("{}m"),
            Colored::BackgroundColor(Color::Reset)
        )
        .unwrap();
        write!(expected, csi!("K")).unwrap();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "\r\n").unwrap();
        // row 2
        write!(expected, csi!("K")).unwrap();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "\r\n").unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_canvas_hyperlink() {
        let mut canvas = Canvas::new(5, 1);
        let url: Arc<str> = Arc::from("http://x");
        {
            let mut sub = canvas.subview_mut(0, 0, 0, 0, 5, 1);
            sub.set_text(0, 0, "link", CanvasTextStyle::default());
            sub.set_hyperlink(0, 0, 4, 1, Some(url.clone()));
        }

        let mut actual = Vec::new();
        canvas.write_ansi(&mut actual).unwrap();

        let mut expected = Vec::new();
        write!(expected, csi!("0m")).unwrap();
        // OSC 8 open, the linked text, then OSC 8 close before clearing the row.
        write!(expected, "\x1b]8;;{url}\x1b\\").unwrap();
        write!(expected, "link").unwrap();
        write!(expected, "\x1b]8;;\x1b\\").unwrap();
        write!(expected, csi!("K")).unwrap();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "\r\n").unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_canvas_full_background_color() {
        let mut canvas = Canvas::new(6, 3);
        assert_eq!(canvas.width(), 6);
        assert_eq!(canvas.height(), 3);

        canvas
            .subview_mut(0, 0, 0, 0, 6, 6)
            .set_background_color(0, 0, 6, 6, Color::Red);

        let mut actual = Vec::new();
        canvas.write_ansi(&mut actual).unwrap();

        // the important thing here is that the background color is reset before each line is
        // cleared and before each newline
        // see: https://github.com/ccbrown/iocraft/issues/142

        let mut expected = Vec::new();

        // line 1
        write!(expected, csi!("0m")).unwrap();
        write!(expected, csi!("{}m"), Colored::BackgroundColor(Color::Red)).unwrap();
        write!(expected, "     ").unwrap();
        write!(
            expected,
            csi!("{}m"),
            Colored::BackgroundColor(Color::Reset)
        )
        .unwrap();
        write!(expected, csi!("K")).unwrap();
        write!(expected, csi!("{}m"), Colored::BackgroundColor(Color::Red)).unwrap();
        write!(expected, " ").unwrap();
        write!(
            expected,
            csi!("{}m"),
            Colored::BackgroundColor(Color::Reset)
        )
        .unwrap();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "\r\n").unwrap();

        // line 2
        write!(expected, csi!("{}m"), Colored::BackgroundColor(Color::Red)).unwrap();
        write!(expected, "     ").unwrap();
        write!(
            expected,
            csi!("{}m"),
            Colored::BackgroundColor(Color::Reset)
        )
        .unwrap();
        write!(expected, csi!("K")).unwrap();
        write!(expected, csi!("{}m"), Colored::BackgroundColor(Color::Red)).unwrap();
        write!(expected, " ").unwrap();
        write!(
            expected,
            csi!("{}m"),
            Colored::BackgroundColor(Color::Reset)
        )
        .unwrap();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "\r\n").unwrap();

        // line 3
        write!(expected, csi!("{}m"), Colored::BackgroundColor(Color::Red)).unwrap();
        write!(expected, "     ").unwrap();
        write!(
            expected,
            csi!("{}m"),
            Colored::BackgroundColor(Color::Reset)
        )
        .unwrap();
        write!(expected, csi!("K")).unwrap();
        write!(expected, csi!("{}m"), Colored::BackgroundColor(Color::Red)).unwrap();
        write!(expected, " ").unwrap();
        write!(
            expected,
            csi!("{}m"),
            Colored::BackgroundColor(Color::Reset)
        )
        .unwrap();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "\r\n").unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_canvas_text_styles() {
        let mut canvas = Canvas::new(100, 1);
        assert_eq!(canvas.width(), 100);
        assert_eq!(canvas.height(), 1);

        canvas
            .subview_mut(0, 0, 0, 0, 1, 1)
            .set_text(0, 0, ".", CanvasTextStyle::default());
        canvas.subview_mut(1, 0, 1, 0, 1, 1).set_text(
            0,
            0,
            ".",
            CanvasTextStyle {
                color: Some(Color::Red),
                weight: Weight::Bold,
                underline: true,
                ..Default::default()
            },
        );
        canvas.subview_mut(2, 0, 2, 0, 1, 1).set_text(
            0,
            0,
            ".",
            CanvasTextStyle {
                color: Some(Color::Red),
                weight: Weight::Bold,
                italic: true,
                ..Default::default()
            },
        );
        canvas.subview_mut(3, 0, 3, 0, 1, 1).set_text(
            0,
            0,
            ".",
            CanvasTextStyle {
                color: Some(Color::Red),
                weight: Weight::Bold,
                ..Default::default()
            },
        );
        canvas.subview_mut(4, 0, 4, 0, 1, 1).set_text(
            0,
            0,
            ".",
            CanvasTextStyle {
                color: Some(Color::Red),
                weight: Weight::Light,
                ..Default::default()
            },
        );
        canvas.subview_mut(5, 0, 5, 0, 1, 1).set_text(
            0,
            0,
            ".",
            CanvasTextStyle {
                color: Some(Color::Red),
                ..Default::default()
            },
        );
        canvas.subview_mut(6, 0, 6, 0, 1, 1).set_text(
            0,
            0,
            ".",
            CanvasTextStyle {
                color: Some(Color::Green),
                ..Default::default()
            },
        );
        canvas.subview_mut(7, 0, 7, 0, 1, 1).set_text(
            0,
            0,
            ".",
            CanvasTextStyle {
                color: Some(Color::Green),
                invert: true,
                ..Default::default()
            },
        );
        canvas.subview_mut(8, 0, 8, 0, 1, 1).set_text(
            0,
            0,
            ".",
            CanvasTextStyle {
                color: Some(Color::Green),
                ..Default::default()
            },
        );

        let mut actual = Vec::new();
        canvas.write_ansi(&mut actual).unwrap();

        let mut expected = Vec::new();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, ".").unwrap();

        write!(expected, csi!("{}m"), Colored::ForegroundColor(Color::Red)).unwrap();
        write!(expected, csi!("{}m"), Attribute::Bold.sgr()).unwrap();
        write!(expected, csi!("{}m"), Attribute::Underlined.sgr()).unwrap();
        write!(expected, ".").unwrap();

        write!(expected, csi!("0m")).unwrap();
        write!(expected, csi!("{}m"), Colored::ForegroundColor(Color::Red)).unwrap();
        write!(expected, csi!("{}m"), Attribute::Bold.sgr()).unwrap();
        write!(expected, csi!("{}m"), Attribute::Italic.sgr()).unwrap();
        write!(expected, ".").unwrap();

        write!(expected, csi!("0m")).unwrap();
        write!(expected, csi!("{}m"), Colored::ForegroundColor(Color::Red)).unwrap();
        write!(expected, csi!("{}m"), Attribute::Bold.sgr()).unwrap();
        write!(expected, ".").unwrap();

        // Bold must be disabled before entering dim-only text.
        write!(expected, csi!("0m")).unwrap();
        write!(expected, csi!("{}m"), Colored::ForegroundColor(Color::Red)).unwrap();
        write!(expected, csi!("{}m"), Attribute::Dim.sgr()).unwrap();
        write!(expected, ".").unwrap();

        write!(expected, csi!("0m")).unwrap();
        write!(expected, csi!("{}m"), Colored::ForegroundColor(Color::Red)).unwrap();
        write!(expected, ".").unwrap();

        write!(
            expected,
            csi!("{}m"),
            Colored::ForegroundColor(Color::Green)
        )
        .unwrap();
        write!(expected, ".").unwrap();

        write!(expected, csi!("{}m"), Attribute::Reverse.sgr()).unwrap();
        write!(expected, ".").unwrap();

        write!(expected, csi!("0m")).unwrap();
        write!(
            expected,
            csi!("{}m"),
            Colored::ForegroundColor(Color::Green)
        )
        .unwrap();
        write!(expected, ".").unwrap();

        write!(expected, csi!("K")).unwrap();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "\r\n").unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_canvas_text_clipping() {
        let mut canvas = Canvas::new(10, 5);
        assert_eq!(canvas.width(), 10);
        assert_eq!(canvas.height(), 5);

        canvas.subview_mut(2, 2, 2, 2, 4, 2).set_text(
            -2,
            -1,
            "line 1\nline 2\nline 3\nline 4",
            CanvasTextStyle::default(),
        );

        let actual = canvas.to_string();
        assert_eq!(actual, "\n\n  ne 2\n  ne 3\n\n");
    }

    #[test]
    fn test_canvas_text_clearing() {
        let mut canvas = Canvas::new(10, 1);
        canvas
            .subview_mut(0, 0, 0, 0, 10, 1)
            .set_text(0, 0, "hello!", CanvasTextStyle::default());
        assert_eq!(canvas.to_string(), "hello!\n");

        canvas.subview_mut(0, 0, 0, 0, 10, 1).clear_text(0, 0, 3, 1);
        assert_eq!(canvas.to_string(), "   lo!\n");
    }

    #[test]
    fn test_write_ansi_without_final_newline() {
        let mut canvas = Canvas::new(10, 3);

        canvas
            .subview_mut(0, 0, 0, 0, 10, 3)
            .set_text(0, 0, "hello!", CanvasTextStyle::default());

        let mut actual = Vec::new();
        canvas
            .write_ansi_without_final_newline(&mut actual)
            .unwrap();

        let mut expected = Vec::new();
        // row 0
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "hello!").unwrap();
        write!(expected, csi!("K")).unwrap();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "\r\n").unwrap();
        // row 1
        write!(expected, csi!("K")).unwrap();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "\r\n").unwrap();
        // row 2 (final, no newline)
        write!(expected, csi!("K")).unwrap();
        write!(expected, csi!("0m")).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_ansi_erase_for_full_rows() {
        let mut canvas = Canvas::new(10, 1);

        canvas.subview_mut(0, 0, 0, 0, 10, 1).set_text(
            0,
            0,
            "1234512345",
            CanvasTextStyle::default(),
        );

        let mut actual = Vec::new();
        canvas.write_ansi(&mut actual).unwrap();

        let mut expected = Vec::new();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "123451234").unwrap();
        write!(expected, csi!("K")).unwrap();
        write!(expected, "5").unwrap();
        write!(expected, csi!("0m")).unwrap();
        write!(expected, "\r\n").unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cell_read() {
        let mut canvas = Canvas::new(10, 3);
        canvas
            .subview_mut(0, 0, 0, 0, 10, 3)
            .set_text(0, 0, "hello", CanvasTextStyle::default());
        assert_eq!(canvas.cell(0, 0).and_then(|c| c.text()), Some("h"));
        assert_eq!(canvas.cell(4, 0).and_then(|c| c.text()), Some("o"));
        assert_eq!(canvas.cell(5, 0).and_then(|c| c.text()), None);
        assert_eq!(canvas.cell(99, 99), None);
    }

    #[test]
    fn test_get_text_single_row() {
        let mut canvas = Canvas::new(10, 3);
        {
            let mut sv = canvas.subview_mut(0, 0, 0, 0, 10, 3);
            sv.set_text(0, 0, "hello", CanvasTextStyle::default());
            sv.set_text(2, 1, "ab", CanvasTextStyle::default());
        }
        assert_eq!(canvas.get_text(0, 0, 10, 1), "hello");
        assert_eq!(canvas.get_text(0, 1, 10, 1), "  ab");
        assert_eq!(canvas.get_text(0, 2, 10, 1), "");
    }

    #[test]
    fn test_get_text_multi_row() {
        let mut canvas = Canvas::new(10, 3);
        {
            let mut sv = canvas.subview_mut(0, 0, 0, 0, 10, 3);
            sv.set_text(0, 0, "line one", CanvasTextStyle::default());
            sv.set_text(0, 1, "line two", CanvasTextStyle::default());
        }
        assert_eq!(
            canvas.get_text(0, 0, 10, 3),
            "line one
line two
"
        );
    }

    #[test]
    fn test_get_text_partial_row() {
        let mut canvas = Canvas::new(10, 1);
        canvas
            .subview_mut(0, 0, 0, 0, 10, 1)
            .set_text(0, 0, "abcdef", CanvasTextStyle::default());
        assert_eq!(canvas.get_text(2, 0, 3, 1), "cde");
    }

    #[test]
    fn test_cell_text_style() {
        let mut canvas = Canvas::new(10, 1);
        let style = CanvasTextStyle {
            weight: Weight::Bold,
            invert: true,
            ..Default::default()
        };
        canvas
            .subview_mut(0, 0, 0, 0, 10, 1)
            .set_text(0, 0, "hi", style);
        let cell = canvas.cell(0, 0).unwrap();
        let ts = cell.text_style().unwrap();
        assert_eq!(ts.weight, Weight::Bold);
        assert!(ts.invert);
        // Empty cell returns None.
        assert!(canvas.cell(5, 0).unwrap().text_style().is_none());
    }

    #[test]
    fn test_cloned_canvas_keeps_unicode_and_independent_run_styles() {
        let mut canvas = Canvas::new(12, 1);
        let first = CanvasTextStyle {
            color: Some(Color::Blue),
            weight: Weight::Bold,
            ..Default::default()
        };
        let second = CanvasTextStyle {
            color: Some(Color::Green),
            dim: true,
            ..Default::default()
        };
        let text = "Ae\u{301}界";
        {
            let mut view = canvas.subview_mut(0, 0, 0, 0, 12, 1);
            view.set_text(0, 0, text, first);
            view.set_text(6, 0, text, second);
        }
        let saved = canvas.clone();
        canvas
            .subview_mut(0, 0, 0, 0, 12, 1)
            .set_text(0, 0, text, second);

        for (offset, glyph) in [(0, "A"), (1, "e\u{301}"), (2, "界")] {
            let original = saved.cell(offset, 0).unwrap();
            let other_run = saved.cell(offset + 6, 0).unwrap();
            let updated = canvas.cell(offset, 0).unwrap();
            assert_eq!(original.text(), Some(glyph));
            assert_eq!(other_run.text(), Some(glyph));
            assert_eq!(updated.text(), Some(glyph));
            assert_eq!(original.text_style(), Some(&first));
            assert_eq!(other_run.text_style(), Some(&second));
            assert_eq!(updated.text_style(), Some(&second));
        }
    }

    #[test]
    fn test_cell_is_empty() {
        let mut canvas = Canvas::new(5, 1);
        assert!(canvas.cell(0, 0).unwrap().is_empty());
        canvas
            .subview_mut(0, 0, 0, 0, 5, 1)
            .set_text(0, 0, "a", CanvasTextStyle::default());
        assert!(!canvas.cell(0, 0).unwrap().is_empty());
    }

    #[test]
    fn test_subview_cell_relative_coords() {
        let mut canvas = Canvas::new(10, 5);
        // Subview at offset (2, 1) with clip matching subview area
        let mut sv = canvas.subview_mut(2, 1, 2, 1, 6, 3);
        sv.set_text(0, 0, "abc", CanvasTextStyle::default());
        // Read back via subview using relative coordinates
        assert_eq!(sv.cell(0, 0).and_then(|c| c.text()), Some("a"));
        assert_eq!(sv.cell(2, 0).and_then(|c| c.text()), Some("c"));
        // Out of clip bounds → None
        assert_eq!(sv.cell(-1, 0), None);
        assert_eq!(sv.cell(6, 0), None);
        assert_eq!(sv.cell(0, -1), None);
        assert_eq!(sv.cell(0, 3), None);
    }

    #[test]
    fn test_subview_get_text_relative_coords() {
        let mut canvas = Canvas::new(10, 5);
        let mut sv = canvas.subview_mut(2, 1, 2, 1, 6, 3);
        sv.set_text(0, 0, "hello", CanvasTextStyle::default());
        sv.set_text(0, 1, "world", CanvasTextStyle::default());
        // Read back relative to subview origin
        assert_eq!(sv.get_text(0, 0, 6, 1), "hello");
        assert_eq!(sv.get_text(0, 0, 6, 2), "hello\nworld");
    }

    #[test]
    fn test_row_eq_same_content() {
        let mut a = Canvas::new(10, 2);
        let mut b = Canvas::new(10, 2);
        a.subview_mut(0, 0, 0, 0, 10, 2)
            .set_text(0, 0, "hello", CanvasTextStyle::default());
        b.subview_mut(0, 0, 0, 0, 10, 2)
            .set_text(0, 0, "hello", CanvasTextStyle::default());

        assert!(a.row_eq(&b, 0));
        assert!(a.row_eq(&b, 1));
    }

    #[test]
    fn test_row_eq_different_content() {
        let mut a = Canvas::new(10, 1);
        let mut b = Canvas::new(10, 1);
        a.subview_mut(0, 0, 0, 0, 10, 1)
            .set_text(0, 0, "hello", CanvasTextStyle::default());
        b.subview_mut(0, 0, 0, 0, 10, 1)
            .set_text(0, 0, "world", CanvasTextStyle::default());

        assert!(!a.row_eq(&b, 0));
    }

    #[test]
    fn test_row_eq_different_widths() {
        let mut a = Canvas::new(10, 1);
        let mut b = Canvas::new(20, 1);
        a.subview_mut(0, 0, 0, 0, 10, 1)
            .set_text(0, 0, "hello", CanvasTextStyle::default());
        b.subview_mut(0, 0, 0, 0, 20, 1)
            .set_text(0, 0, "hello", CanvasTextStyle::default());

        assert!(!a.row_eq(&b, 0));
    }

    #[test]
    fn test_row_eq_out_of_bounds() {
        let a = Canvas::new(10, 1);
        let b = Canvas::new(10, 2);

        // row 1 is out of bounds for a, but exists (empty) in b
        assert!(!a.row_eq(&b, 1));
    }

    #[test]
    fn test_write_ansi_row_without_newline() {
        let mut canvas = Canvas::new(10, 2);
        canvas
            .subview_mut(0, 0, 0, 0, 10, 2)
            .set_text(0, 0, "hello", CanvasTextStyle::default());
        canvas
            .subview_mut(0, 0, 0, 0, 10, 2)
            .set_text(0, 1, "world", CanvasTextStyle::default());

        // Each row renders without a leading reset (caller's contract is to
        // provide clean SGR state) but always leaves SGR state reset on return.
        let mut row0 = Vec::new();
        canvas.write_ansi_row_without_newline(0, &mut row0).unwrap();

        let mut expected0 = Vec::new();
        write!(expected0, "hello").unwrap();
        write!(expected0, csi!("K")).unwrap();
        write!(expected0, csi!("0m")).unwrap();
        assert_eq!(row0, expected0);

        let mut row1 = Vec::new();
        canvas.write_ansi_row_without_newline(1, &mut row1).unwrap();

        let mut expected1 = Vec::new();
        write!(expected1, "world").unwrap();
        write!(expected1, csi!("K")).unwrap();
        write!(expected1, csi!("0m")).unwrap();
        assert_eq!(row1, expected1);
    }
}
