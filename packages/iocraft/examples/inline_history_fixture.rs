//! Real-PTY fixture: independent history hooks must share terminal ownership.
use iocraft::prelude::*;

#[component]
fn Fixture(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let (width, height) = hooks.use_terminal_size();
    let (first, first_error) = hooks.use_output();
    let (second, _) = hooks.use_output();
    let mut phase = hooks.use_state(|| 0_u32);
    let mut acknowledgment = hooks.use_state(|| 0_u32);
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
                KeyCode::F(2) => {
                    first.print("SharedPartial");
                    phase.set(2);
                }
                KeyCode::F(3) => {
                    second.println("Joined");
                    phase.set(3);
                }
                KeyCode::F(4) => {
                    first.print("SameBatch:");
                    second.println("end");
                    phase.set(4);
                }
                KeyCode::F(5) => {
                    first.print("StdoutPrefix:");
                    first_error.println("StderrSuffix");
                    phase.set(5);
                }
                KeyCode::F(6) => acknowledgment.set(acknowledgment.get() + 1),
                KeyCode::Esc => done.set(true),
                _ => {}
            }
        }
    });
    if done.get() {
        system.exit();
    }
    element! {
        View(flex_direction: FlexDirection::Column) {
            Text(content: format!("StatusMarker phase={} {}", phase.get(), "long status ".repeat(8)))
            Text(content: "TodoMarker 1 todos (1 done, 0 open)")
            Text(content: "─".repeat(width as usize))
            View(flex_direction: FlexDirection::Row) {
                Text(content: "❯ ")
                TextInput(value: value.to_string(), has_focus: true, on_change: move |v| value.set(v))
            }
            Text(content: "─".repeat(width as usize))
            Text(content: format!("FooterMarker p{} a{} {width}x{height}", phase.get(), acknowledgment.get()))
        }
    }
}

fn main() {
    println!("ShellHistoryBeforeScode");
    for index in 0..70 {
        println!("Earlier history line {index}");
    }
    smol::block_on(element!(Fixture).render_loop()).unwrap();
}
