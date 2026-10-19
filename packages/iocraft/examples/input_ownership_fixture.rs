//! Controlled-input diagnostic: parent commands and child edits in one burst.
use iocraft::prelude::*;

fn trace_input_event(event: &TerminalEvent, width: u16, height: u16) {
    let Ok(path) = std::env::var("IOCRAFT_INPUT_EVENT_TRACE") else {
        return;
    };
    // Optional fixture-only observation, never an input-admission mechanism.
    use std::io::Write;
    if let Ok(mut log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let physical = crossterm::terminal::size().ok();
        let _ = writeln!(
            log,
            "callback={width}x{height} physical={physical:?} event={event:?}"
        );
    }
}

#[component]
fn Fixture(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let (width, height) = hooks.use_terminal_size();
    let input_frame = hooks.use_input_frame();
    let reviewable = width >= 40 && height >= 12;
    input_frame.set_enabled(reviewable);
    let mut value = hooks.use_state(String::new);
    let mut exit = hooks.use_state(|| false);
    let mut acknowledgment = hooks.use_state(|| 0_u32);
    let mut confirmed = hooks.use_state(|| 0_u32);
    let mut system = hooks.use_context_mut::<SystemContext>();
    hooks.use_terminal_events(move |event| {
        trace_input_event(&event, width, height);
        if let TerminalEvent::Key(KeyEvent {
            code,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match code {
                KeyCode::Esc => exit.set(true),
                KeyCode::F(6) => acknowledgment.set(acknowledgment.get() + 1),
                KeyCode::Enter if reviewable && input_frame.is_current() => {
                    confirmed.set(confirmed.get() + 1)
                }
                _ => {}
            }
        }
    });
    if exit.get() {
        system.exit();
    }
    let on_edit: TextInputEditHandler = Box::new(|event, current, cursor| match event {
        TerminalEvent::Key(KeyEvent {
            code: KeyCode::Char('u'),
            modifiers,
            ..
        }) if modifiers.contains(KeyModifiers::CONTROL) => Some(TextInputEdit {
            value: String::new(),
            cursor_offset: 0,
        }),
        TerminalEvent::Paste(text) => Some(TextInputEdit {
            value: format!("{}[{}]{}", &current[..cursor], text, &current[cursor..]),
            cursor_offset: cursor + text.len() + 2,
        }),
        _ => None,
    });
    element! {
        View(flex_direction: FlexDirection::Column) {
            Text(content: format!("Viewport:{width}x{height}:{}", if reviewable { "reviewable" } else { "blocked" }))
            Text(content: if reviewable { "Focus:on" } else { "Focus:off" })
            Text(content: format!("InputValue:{}:EndValue", value.read().as_str()))
            Text(content: format!("BatchAck:{}", acknowledgment.get()))
            Text(content: format!("Confirmed:{}", confirmed.get()))
            View(width: 80, height: 1) {
                TextInput(
                    value: value.to_string(),
                    has_focus: reviewable,
                    on_change: move |text| value.set(text),
                    on_edit: Some(on_edit),
                )
            }
        }
    }
}

fn main() {
    smol::block_on(element!(Fixture).render_loop()).unwrap();
}
