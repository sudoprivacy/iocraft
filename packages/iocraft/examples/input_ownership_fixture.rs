//! Controlled-input diagnostic: parent commands and child edits in one burst.
use iocraft::prelude::*;

#[component]
fn Fixture(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let mut value = hooks.use_state(String::new);
    let mut exit = hooks.use_state(|| false);
    let mut acknowledgment = hooks.use_state(|| 0_u32);
    let mut system = hooks.use_context_mut::<SystemContext>();
    hooks.use_terminal_events(move |event| {
        if let TerminalEvent::Key(KeyEvent {
            code,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match code {
                KeyCode::Esc => exit.set(true),
                KeyCode::F(6) => acknowledgment.set(acknowledgment.get() + 1),
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
            Text(content: format!("InputValue:{}:EndValue", value.read().as_str()))
            Text(content: format!("BatchAck:{}", acknowledgment.get()))
            View(width: 80, height: 1) {
                TextInput(
                    value: value.to_string(),
                    has_focus: true,
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
