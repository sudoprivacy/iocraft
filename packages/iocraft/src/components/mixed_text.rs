use crate::{
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
        self.contents = props.contents.clone();
        self.wrap = props.wrap;
        self.align = props.align;
        updater.set_measure_func(Text::measure_func(plaintext, props.wrap));
    }

    fn draw(&mut self, drawer: &mut ComponentDrawer<'_>) {
        let width = drawer.layout().size.width;
        let segmented_string: SegmentedString = self
            .contents
            .iter()
            .map(|content| content.text.as_str())
            .collect();
        let lines = segmented_string.wrap(match self.wrap {
            TextWrap::Wrap => width as usize,
            TextWrap::NoWrap => usize::MAX,
        });

        let paddings = lines
            .iter()
            .map(|line| Text::alignment_padding(line.width, self.align, width as _))
            .collect::<Vec<_>>();
        let x_offset = paddings.iter().copied().min().unwrap_or(0);

        let mut drawer = TextDrawer::new(drawer, x_offset, self.align != TextAlign::Left);
        for (mut line, padding) in lines.into_iter().zip(paddings) {
            // Painted trailing cells are part of a diff/selection surface.
            // Trimming them would erase its fill on shorter wrapped rows.
            let painted_tail = line
                .segments
                .last()
                .is_some_and(|segment| self.contents[segment.index].background_color.is_some());
            if self.wrap == TextWrap::Wrap && !painted_tail {
                line.trim_end();
            }

            let additional_padding = padding - x_offset;
            if additional_padding > 0 {
                drawer.append_lines(
                    [format!("{:width$}", "", width = additional_padding as usize).as_str()],
                    CanvasTextStyle::default(),
                    None,
                );
            }
            let mut segments = line.segments.into_iter().peekable();
            while let Some(segment) = segments.next() {
                let content = &self.contents[segment.index];
                let style = CanvasTextStyle {
                    color: content.color,
                    weight: content.weight,
                    dim: content.dim,
                    strikethrough: content.strikethrough,
                    underline: content.decoration == TextDecoration::Underline,
                    italic: content.italic,
                    invert: content.invert,
                };
                if segments.peek().is_some() {
                    drawer.append_lines_with_background(
                        [segment.text],
                        style,
                        content.hyperlink.as_deref(),
                        content.background_color,
                    );
                } else {
                    drawer.append_lines_with_background(
                        [segment.text, ""],
                        style,
                        content.hyperlink.as_deref(),
                        content.background_color,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;

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
            View(width: 4) { MixedText(contents: vec![padded]) }
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
