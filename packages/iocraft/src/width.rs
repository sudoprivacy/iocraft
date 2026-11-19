//! Terminal columns of text, by one rule shared by placement, wrapping, the
//! text input caret and the application.
//!
//! Terminals disagree on how wide they draw a grapheme cluster made of several
//! code points. iTerm2, kitty, WezTerm, Ghostty and Windows Terminal draw
//! `👩🏽‍💻` as one two-cell cluster ([`GraphemeWidth::Clustered`], the rule of
//! `unicode-width`). xterm.js with Unicode 11, which VS Code uses, lays out each
//! code point and draws it six cells wide ([`GraphemeWidth::PerCodePoint`]).
//! When the layout and the terminal disagree, everything after such a cluster
//! lands in the wrong cells and the caret is drawn beside the text it belongs
//! to. An application measures its terminal once and calls
//! [`set_grapheme_width`]; every width here follows it.

use std::sync::atomic::{AtomicU8, Ordering};

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// How the terminal draws a grapheme cluster of several code points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphemeWidth {
    /// The cluster is drawn as one glyph, as wide as `unicode-width` says.
    #[default]
    Clustered,
    /// Each code point is laid out on its own, so the cluster is as wide as
    /// the sum of its code points.
    PerCodePoint,
}

static MODE: AtomicU8 = AtomicU8::new(0);

#[cfg(test)]
thread_local! {
    static TEST_MODE: std::cell::Cell<Option<GraphemeWidth>> = const { std::cell::Cell::new(None) };
}

/// Sets how the terminal draws grapheme clusters, for every width that
/// follows. Call it once, before the first frame.
pub fn set_grapheme_width(mode: GraphemeWidth) {
    MODE.store(mode as u8, Ordering::Relaxed);
}

/// How the terminal draws grapheme clusters.
pub fn grapheme_width() -> GraphemeWidth {
    #[cfg(test)]
    if let Some(mode) = TEST_MODE.with(std::cell::Cell::get) {
        return mode;
    }
    if MODE.load(Ordering::Relaxed) == GraphemeWidth::PerCodePoint as u8 {
        GraphemeWidth::PerCodePoint
    } else {
        GraphemeWidth::Clustered
    }
}

/// Runs `f` with `mode` on this thread only, so tests of either rule can run
/// in parallel.
#[cfg(test)]
pub(crate) fn with_grapheme_width<T>(mode: GraphemeWidth, f: impl FnOnce() -> T) -> T {
    let previous = TEST_MODE.with(|cell| cell.replace(Some(mode)));
    let result = f();
    TEST_MODE.with(|cell| cell.set(previous));
    result
}

/// Columns one grapheme cluster occupies. Controls occupy none.
pub fn grapheme_columns(grapheme: &str) -> usize {
    if grapheme.chars().all(char::is_control) {
        return 0;
    }
    // Terminals draw the halfwidth voiced and semi-voiced sound marks in a
    // cell of their own. `unicode-width` releases disagree on whether they
    // have a width (0.2.2 says none, 0.2.3 one), so count them here.
    if grapheme.contains(['\u{FF9E}', '\u{FF9F}']) {
        return grapheme
            .chars()
            .map(|c| match c {
                '\u{FF9E}' | '\u{FF9F}' => 1,
                _ => c.width().unwrap_or(0),
            })
            .sum();
    }
    match grapheme_width() {
        GraphemeWidth::Clustered => grapheme.width(),
        GraphemeWidth::PerCodePoint => grapheme.chars().map(|c| c.width().unwrap_or(0)).sum(),
    }
}

/// Columns of `text`: the sum of its grapheme clusters' columns.
pub fn str_columns(text: &str) -> usize {
    if text.is_ascii() {
        return text
            .bytes()
            .filter(|byte| (0x20..0x7f).contains(byte))
            .count();
    }
    text.graphemes(true).map(grapheme_columns).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKER: &str = "\u{1F469}\u{1F3FD}\u{200D}\u{1F4BB}";

    #[test]
    fn a_zwj_cluster_follows_the_terminal_rule() {
        with_grapheme_width(GraphemeWidth::Clustered, || {
            assert_eq!(grapheme_columns(WORKER), 2);
            assert_eq!(str_columns(&format!("ab{WORKER}XY")), 6);
        });
        with_grapheme_width(GraphemeWidth::PerCodePoint, || {
            assert_eq!(grapheme_columns(WORKER), 6);
            assert_eq!(str_columns(&format!("ab{WORKER}XY")), 10);
        });
    }

    #[test]
    fn single_code_points_agree_under_both_rules() {
        for mode in [GraphemeWidth::Clustered, GraphemeWidth::PerCodePoint] {
            with_grapheme_width(mode, || {
                assert_eq!(str_columns("abc"), 3);
                assert_eq!(str_columns("中文"), 4);
                assert_eq!(str_columns("e\u{301}"), 1, "a combining mark adds nothing");
                assert_eq!(str_columns("a\tb\n"), 2, "controls occupy no column");
                assert_eq!(
                    str_columns("\u{FF8A}\u{FF9F}"),
                    2,
                    "a sound mark takes a cell"
                );
            });
        }
    }
}
