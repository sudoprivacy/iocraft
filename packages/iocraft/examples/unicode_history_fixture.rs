//! Real-terminal reduction of input ownership after a Unicode history handoff.
use iocraft::prelude::*;

fn document() -> String {
    match std::env::var("IOCRAFT_UNICODE_HISTORY_CASE").as_deref() {
        Ok("combining") => format!(
            "• WidthStart 中文 e\u{301} {} WidthDone",
            "ordinary words ".repeat(14)
        ),
        Ok("laptop") => "• ProseStart 中文 e\u{301} 💻 ordinary text with bold words, inline code, and a link (https://example.com). More ordinary prose wraps across several rows while the input stays available. ProseDone.".to_string(),
        Ok("ascii") => format!("• WidthStart plain {} WidthDone", "ordinary words ".repeat(14)),
        other => panic!("explicit fixture case required: {other:?}"),
    }
}

#[component]
fn Fixture(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let (width, height) = hooks.use_terminal_size();
    let (history, _) = hooks.use_output();
    let mut phase = hooks.use_state(|| 0_u32);
    let mut value = hooks.use_state(String::new);
    let mut done = hooks.use_state(|| false);
    let mut system = hooks.use_context_mut::<SystemContext>();
    hooks.use_terminal_events(move |event| {
        if let TerminalEvent::Key(KeyEvent {
            code,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match code {
                KeyCode::F(1) => {
                    history.print(format!("❯ {}\n\n", "submitted prompt ".repeat(10)));
                    phase.set(1);
                }
                KeyCode::F(2) => {
                    history.print(format!("{}\n\n[fixture-model] turn 1\n\n", document()));
                    phase.set(2);
                }
                KeyCode::Esc => done.set(true),
                _ => {}
            }
        }
    });
    if done.get() {
        system.exit();
    }
    let preview = phase.get() == 1;
    element! {
        View(width: u32::from(width), flex_direction: FlexDirection::Column) {
            View(height: if preview { 1 } else { 0 }, flex_direction: FlexDirection::Column) {
                #(preview.then(|| element! { Text(content: "• ProseStart 中", wrap: TextWrap::NoWrap) }))
            }
            View(height: if preview { 2 } else { 0 }) {
                #(preview.then(|| element! { Text(content: "Working...") }))
            }
            Text(content: "─".repeat(usize::from(width)), wrap: TextWrap::NoWrap)
            View(flex_direction: FlexDirection::Row, height: 1) {
                Text(content: "❯ ")
                TextInput(value: value.to_string(), has_focus: true, on_change: move |text| value.set(text))
            }
            Text(content: "─".repeat(usize::from(width)), wrap: TextWrap::NoWrap)
            Text(content: format!("UnicodeFooter p{} {width}x{height}", phase.get()))
        }
    }
}

fn main() {
    let mut prefix = String::new();
    for row in 0..19 {
        prefix.push_str(&format!("Initial history {row}\n"));
    }
    print!("{prefix}");
    if let Ok(path) = std::env::var("IOCRAFT_FRAME_WRITE_TRACE") {
        use std::io::Write;
        writeln!(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap(),
            "startup={prefix:?}"
        )
        .unwrap();
    }
    smol::block_on(element!(Fixture).render_loop()).unwrap();
}
