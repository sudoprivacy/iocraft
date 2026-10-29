use crate::{
    canvas::CanvasRows,
    components::text::{Text, TextAlign, TextDecoration, TextDrawer, TextWrap},
    segmented_string::SegmentedString,
    strip_ansi::strip_ansi,
    CanvasTextStyle, Color, Component, ComponentDrawer, ComponentUpdater, Hooks, Props, Weight,
};

/// A section of text in a [`MixedText`] component.
#[non_exhaustive]
#[derive(Default, Clone)]
pub struct MixedTextContent {
    /// The text to display.
    pub text: String,

    /// The color to make the text.
    pub color: Option<Color>,

    /// The background color behind this section's terminal cells.
    pub background_color: Option<Color>,

    /// The weight of the text.
    pub weight: Weight,

    /// Whether to dim the text, independently of its weight.
    pub dim: bool,

    /// Whether to draw a strikethrough through the text.
    pub strikethrough: bool,

    /// The text decoration.
    pub decoration: TextDecoration,

    /// Whether to italicize the text.
    pub italic: bool,

    /// Whether to invert the text's foreground and background colors.
    pub invert: bool,

    /// The OSC 8 hyperlink target, if the text is a link.
    pub hyperlink: Option<String>,
}

impl MixedTextContent {
    /// Creates a new [`MixedTextContent`] with the given text.
    pub fn new<S: ToString>(text: S) -> Self {
        Self {
            text: text.to_string(),
            ..Default::default()
        }
    }

    /// Returns a new [`MixedTextContent`] with the given color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Returns a new [`MixedTextContent`] with the given weight.
    pub fn weight(mut self, weight: Weight) -> Self {
        self.weight = weight;
        self
    }

    /// Returns a new [`MixedTextContent`] with dim text, preserving its weight.
    pub fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    /// Returns a new [`MixedTextContent`] with strikethrough text.
    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    /// Returns a new [`MixedTextContent`] with the given text decoration.
    pub fn decoration(mut self, decoration: TextDecoration) -> Self {
        self.decoration = decoration;
        self
    }

    /// Returns a new [`MixedTextContent`] with italic text.
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// Returns a new [`MixedTextContent`] with inverted foreground and background colors.
    pub fn invert(mut self) -> Self {
        self.invert = true;
        self
    }

    /// Returns a new [`MixedTextContent`] wrapped in the given OSC 8 hyperlink.
    pub fn hyperlink(mut self, url: impl ToString) -> Self {
        self.hyperlink = Some(url.to_string());
        self
    }
}

/// The props which can be passed to the [`MixedText`] component.
#[non_exhaustive]
#[derive(Default, Props)]
pub struct MixedTextProps {
    /// The contents of the text.
    pub contents: Vec<MixedTextContent>,

    /// The text wrapping behavior.
    pub wrap: TextWrap,

    /// The text alignment.
    pub align: TextAlign,
}

/// `MixedText` is a component that renders a text string containing a mix of styles.
///
/// If you want to render a text string with a single style, use the [`Text`] component instead.
///
/// # Example
///
/// ```
/// # use iocraft::prelude::*;
/// # fn my_element() -> impl Into<AnyElement<'static>> {
/// element! {
///     View(
///         border_style: BorderStyle::Round,
///         border_color: Color::Blue,
///         width: 30,
///     ) {
///         MixedText(align: TextAlign::Center, contents: vec![
///             MixedTextContent::new("Hello, world!").color(Color::Red).weight(Weight::Bold),
///             MixedTextContent::new(" Lorem ipsum odor amet, consectetuer adipiscing elit.").color(Color::Green),
///         ])
///     }
/// }
/// # }
/// ```
#[derive(Default)]
pub struct MixedText {
    contents: Vec<MixedTextContent>,
    wrap: TextWrap,
    align: TextAlign,
    paint_layout: Option<PaintLayout>,
}

// Keep byte ranges into `contents`, so retained components can repaint without
// repeating Unicode line breaking or retaining a second copy of the text.
struct PaintLayout {
    width: usize,
    x_offset: isize,
    lines: Vec<PaintLine>,
    raster: Option<CanvasRows>,
}

struct PaintSpan {
    content: usize,
    range: std::ops::Range<usize>,
}
struct PaintLine {
    padding: isize,
    spans: Vec<PaintSpan>,
}

impl Component for MixedText {
    type Props<'a> = MixedTextProps;

    fn new(_props: &Self::Props<'_>) -> Self {
        Self::default()
    }

    fn update(
        &mut self,
        props: &mut Self::Props<'_>,
        _hooks: Hooks,
        updater: &mut ComponentUpdater,
    ) {
        for content in props.contents.iter_mut() {
            content.text = strip_ansi(&content.text).into_owned();
        }
        let plaintext = props
            .contents
            .iter()
            .map(|content| content.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        // Ranges and alignment belong to these exact contents and wrap mode.
        self.paint_layout = None;
        self.contents = props.contents.clone();
        self.wrap = props.wrap;
        self.align = props.align;
        updater.set_measure_func(Text::measure_func(plaintext, props.wrap));
    }

    fn draw(&mut self, drawer: &mut ComponentDrawer<'_>) {
        let width = drawer.layout().size.width as usize;
        if self
            .paint_layout
            .as_ref()
            .is_none_or(|layout| layout.width != width)
        {
            let text: SegmentedString = self
                .contents
                .iter()
                .map(|content| content.text.as_str())
                .collect();
            let lines = text.wrap(if self.wrap == TextWrap::Wrap {
                width
            } else {
                usize::MAX
            });
            let mut prepared = Vec::with_capacity(lines.len());
            for mut line in lines {
                let padding = Text::alignment_padding(line.width, self.align, width);
                // Painted trailing cells are part of a diff/selection surface.
                // Trimming them would erase its fill on shorter wrapped rows.
                let painted_tail = line
                    .segments
                    .iter()
                    .rfind(|span| !span.text.is_empty())
                    .is_some_and(|span| self.contents[span.index].background_color.is_some());
                if self.wrap == TextWrap::Wrap && !painted_tail {
                    line.trim_end();
                }
                let spans = line
                    .segments
                    .into_iter()
                    .map(|span| PaintSpan {
                        content: span.index,
                        range: span.offset..span.offset + span.text.len(),
                    })
                    .collect();
                prepared.push(PaintLine { padding, spans });
            }
            let x_offset = prepared.iter().map(|line| line.padding).min().unwrap_or(0);
            self.paint_layout = Some(PaintLayout {
                width,
                x_offset,
                lines: prepared,
                raster: None,
            });
        }
        let layout = self.paint_layout.as_ref().expect("prepared paint layout");
        if let Some(rows) = &layout.raster {
            if drawer.canvas().restore_rows(rows) {
                return;
            }
        }
        let cacheable = drawer.canvas().has_pristine_rows();
        let mut painter = TextDrawer::new(drawer, layout.x_offset, self.align != TextAlign::Left);
        for line in &layout.lines {
            // Blank rows still occupy measured rows after whitespace trimming.
            if line.spans.is_empty() {
                painter.append_lines(["", ""], CanvasTextStyle::default(), None);
                continue;
            }
            let padding = line.padding - layout.x_offset;
            if padding > 0 {
                painter.append_lines(
                    [" ".repeat(padding as usize).as_str()],
                    CanvasTextStyle::default(),
                    None,
                );
            }
            for (index, span) in line.spans.iter().enumerate() {
                let content = &self.contents[span.content];
                let style = CanvasTextStyle {
                    color: content.color,
                    weight: content.weight,
                    dim: content.dim,
                    strikethrough: content.strikethrough,
                    underline: content.decoration == TextDecoration::Underline,
                    italic: content.italic,
                    invert: content.invert,
                };
                let text = &content.text[span.range.clone()];
                if index + 1 == line.spans.len() {
                    painter.append_lines_with_background(
                        [text, ""],
                        style,
                        content.hyperlink.as_deref(),
                        content.background_color,
                    );
                } else {
                    painter.append_lines_with_background(
                        [text],
                        style,
                        content.hyperlink.as_deref(),
                        content.background_color,
                    );
                }
            }
        }
        if cacheable {
            self.paint_layout
                .as_mut()
                .expect("prepared paint layout")
                .raster = drawer.canvas().capture_rows();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;

    #[test]
    fn nowrap_alignment_clips_long_rows_without_losing_short_rows() {
        for align in [TextAlign::Center, TextAlign::Right] {
            let text = "abcdefghijk\nx";
            let plain = element! {
                View(width: 4) { Text(content: text, wrap: TextWrap::NoWrap, align) }
            }
            .render(None);
            let mixed = element! {
                View(width: 4) {
                    MixedText(contents: vec![MixedTextContent::new(text)], wrap: TextWrap::NoWrap, align)
                }
            }.render(None);
            assert_eq!(mixed.get_text(0, 0, 4, 2), plain.get_text(0, 0, 4, 2));
        }
    }

    #[test]
    fn mixed_text_keeps_blank_rows_in_the_measured_coordinate_space() {
        for text in ["first\n\nlast", "\nfirst\n  \n\nlast", "first\n \nlast\n\n"] {
            for width in [4, 20] {
                let plain = element! { Text(content: text) }.render(Some(width));
                let mixed = element! { MixedText(contents: vec![MixedTextContent::new(text)]) }
                    .render(Some(width));
                assert_eq!(mixed.height(), plain.height());
                for row in 0..plain.height() {
                    assert_eq!(
                        mixed.get_text(0, row, width, 1).trim_end(),
                        plain.get_text(0, row, width, 1).trim_end(),
                        "{text:?}, width {width}, row {row}"
                    );
                }
            }
        }
        let styled = element! { MixedText(contents: vec![
            MixedTextContent::new("first\n").color(Color::Blue),
            MixedTextContent::new("\n \n").weight(Weight::Bold),
            MixedTextContent::new("last").color(Color::Green),
        ]) }
        .render(Some(20));
        assert_eq!(styled.get_text(0, 3, 20, 1).trim_end(), "last");
        assert_eq!(
            styled.cell(0, 3).unwrap().text_style().unwrap().color,
            Some(Color::Green)
        );
    }

    #[test]
    fn mixed_backgrounds_survive_wrap_without_coloring_following_text() {
        let mut added = MixedTextContent::new("abcd");
        added.background_color = Some(Color::Rgb {
            r: 33,
            g: 58,
            b: 43,
        });
        let mut removed = MixedTextContent::new("efgh");
        removed.background_color = Some(Color::Rgb {
            r: 74,
            g: 34,
            b: 29,
        });
        let canvas = element! {
            View(width: 4) {
                MixedText(contents: vec![added, MixedTextContent::new(" "), removed, MixedTextContent::new(" tail")])
            }
        }.render(None);
        assert_eq!(
            canvas.cell(0, 0).unwrap().background_color,
            Some(Color::Rgb {
                r: 33,
                g: 58,
                b: 43
            })
        );
        assert_eq!(
            canvas.cell(0, 1).unwrap().background_color,
            Some(Color::Rgb {
                r: 74,
                g: 34,
                b: 29
            })
        );
        assert_eq!(canvas.cell(0, 2).unwrap().background_color, None);
        assert_eq!(canvas.get_text(0, 2, 4, 1), "tail");
        let mut padded = MixedTextContent::new("ab  ");
        padded.background_color = Some(Color::DarkGreen);
        let canvas = element! {
            View(width: 4) { MixedText(contents: vec![padded, MixedTextContent::new("\ntail")]) }
        }
        .render(None);
        assert_eq!(
            canvas.cell(3, 0).unwrap().background_color,
            Some(Color::DarkGreen)
        );
    }

    #[test]
    fn test_mixed_text() {
        assert_eq!(element!(MixedText).to_string(), "\n");

        assert_eq!(
            element! {
                View(width: 14) {
                    MixedText(contents: vec![
                        MixedTextContent::new("this is ").color(Color::Red).weight(Weight::Bold).italic(),
                        MixedTextContent::new("a wrapping test").decoration(TextDecoration::Underline),
                    ])
                }
            }
            .to_string(),
            "this is a\nwrapping test\n"
        );
    }

    #[test]
    fn test_mixed_text_hyperlink() {
        let mut out = Vec::new();
        element! {
            View(width: 30) {
                MixedText(contents: vec![
                    MixedTextContent::new("click "),
                    MixedTextContent::new("here").hyperlink("http://example.org"),
                ])
            }
        }
        .render(None)
        .write_ansi(&mut out)
        .unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains("\x1b]8;;http://example.org\x1b\\here\x1b]8;;\x1b\\"),
            "must wrap text in OSC 8: {s:?}"
        );
        assert!(s.contains("click"), "non-link text kept: {s:?}");
    }

    #[test]
    fn test_mixed_text_invert() {
        let canvas = element! {
            MixedText(contents: vec![
                MixedTextContent::new("foo").invert(),
            ])
        }
        .render(None);
        assert!(canvas.cell(0, 0).unwrap().text_style().unwrap().invert);
    }

    #[test]
    fn test_mixed_text_independent_attributes_and_transitions() {
        let canvas = element! {
            View(width: 20) {
                MixedText(contents: vec![
                    MixedTextContent::new("A").weight(Weight::Bold).dim().strikethrough(),
                    MixedTextContent::new("B").weight(Weight::Bold),
                    MixedTextContent::new("C").weight(Weight::Light),
                    MixedTextContent::new("D").weight(Weight::Bold),
                    MixedTextContent::new("E").weight(Weight::Bold).dim(),
                    MixedTextContent::new("F").dim(),
                    MixedTextContent::new("G"),
                ])
            }
        }
        .render(None);
        let style = canvas.cell(0, 0).unwrap().text_style().unwrap();
        assert_eq!(style.weight, Weight::Bold);
        assert!(style.dim && style.strikethrough);
        let mut ansi = Vec::new();
        canvas.write_ansi(&mut ansi).unwrap();
        let ansi = String::from_utf8(ansi).unwrap();
        assert!(
            ansi.contains(concat!(
                "\x1b[1m\x1b[2m\x1b[9mA", // combined attributes
                "\x1b[0m\x1b[1mB",        // dim + strike removed
                "\x1b[0m\x1b[2mC",        // legacy Light is dim-only
                "\x1b[0m\x1b[1mD",        // legacy Light -> Bold clears dim
                "\x1b[2mE",               // add dim while keeping bold
                "\x1b[0m\x1b[2mF",        // remove bold while keeping dim
                "\x1b[0mG",               // no attributes leak
            )),
            "{ansi:?}"
        );
    }

    #[test]
    fn test_strikethrough_does_not_cover_empty_padding() {
        let mut node = element! {
            View(width: 8) {
                MixedText(contents: vec![MixedTextContent::new("done").strikethrough()])
            }
        };
        let mut canvas = node.render(None);
        canvas
            .subview_mut(0, 0, 0, 0, 8, 1)
            .set_text(6, 0, "X", CanvasTextStyle::default());
        let mut bytes = Vec::new();
        canvas.write_ansi(&mut bytes).unwrap();
        let ansi = String::from_utf8(bytes).unwrap();
        assert!(ansi.contains("\x1b[9mdone\x1b[0m"), "{ansi:?}");
    }

    #[test]
    fn test_mixed_text_strips_ansi() {
        assert_eq!(
            element! {
                View(width: 14) {
                    MixedText(contents: vec![
                        MixedTextContent::new("\x1b[31mthis is \x1b[0m"),
                        MixedTextContent::new("\x1b[1ma wrapping test\x1b[0m"),
                    ])
                }
            }
            .to_string(),
            "this is a\nwrapping test\n"
        );
    }
}
