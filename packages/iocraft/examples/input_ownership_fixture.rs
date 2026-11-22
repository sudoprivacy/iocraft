//! Controlled-input diagnostic: parent commands and child edits in one burst.
use iocraft::prelude::*;

// Hold only this fixture's paint while the driver restores the viewport.
// The native queue gets the same VT records as a bracketed paste. Writing
// them directly separates the application's scheduling from pipe backpressure.
#[cfg(windows)]
fn queue_paste_before_restore() {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, WriteConsoleInputW, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT,
        KEY_EVENT_RECORD, KEY_EVENT_RECORD_0, STD_INPUT_HANDLE,
    };

    let Some(gate) = std::env::var_os("IOCRAFT_INPUT_QUEUE_GATE").map(PathBuf::from) else {
        return;
    };
    let size = crossterm::terminal::size().unwrap();
    let records: Vec<_> = "\x1b[200~QueuedPaste\x1b[201~"
        .encode_utf16()
        .map(|character| INPUT_RECORD {
            EventType: KEY_EVENT as u16,
            Event: INPUT_RECORD_0 {
                KeyEvent: KEY_EVENT_RECORD {
                    bKeyDown: 1,
                    wRepeatCount: 1,
                    wVirtualKeyCode: 0,
                    wVirtualScanCode: 0,
                    uChar: KEY_EVENT_RECORD_0 {
                        UnicodeChar: character,
                    },
                    dwControlKeyState: 0,
                },
            },
        })
        .collect();
    let mut written = 0;
    // SAFETY: the fixture's input handle is its own attached console. The
    // records are initialized and remain live for the synchronous call.
    let result = unsafe {
        WriteConsoleInputW(
            GetStdHandle(STD_INPUT_HANDLE),
            records.as_ptr(),
            records.len() as u32,
            &mut written,
        )
    };
    assert_ne!(result, 0, "{}", std::io::Error::last_os_error());
    assert_eq!(written as usize, records.len());
    std::fs::write(
        gate.join("queued"),
        format!("{}x{} {written}", size.0, size.1),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if gate.join("release").is_file() && crossterm::terminal::size().unwrap() == (100, 30) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fixture was not restored and released"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(not(windows))]
fn queue_paste_before_restore() {}

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
    let mut observed_queue = hooks.use_ref_default::<bool>();
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
    if acknowledgment.get() == 1 && !*observed_queue.read() {
        *observed_queue.write() = true;
        queue_paste_before_restore();
    }
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
