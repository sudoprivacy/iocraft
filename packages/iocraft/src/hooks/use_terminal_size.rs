use crate::{
    hooks::{UseState, UseTerminalEvents},
    Hooks, TerminalEvent,
};
use crossterm::terminal;

mod private {
    pub trait Sealed {}
    impl Sealed for crate::Hooks<'_, '_> {}
}

/// `UseTerminalSize` is a hook that returns the current terminal size.
pub trait UseTerminalSize: private::Sealed {
    /// Returns the current terminal size as a tuple of `(width, height)`.
    fn use_terminal_size(&mut self) -> (u16, u16);
}

impl UseTerminalSize for Hooks<'_, '_> {
    fn use_terminal_size(&mut self) -> (u16, u16) {
        let frame_size = self
            .context_stack
            .and_then(|stack| stack.get_context::<crate::context::TerminalSizeSnapshot>())
            .and_then(|snapshot| snapshot.0);
        let mut size =
            self.use_state(|| frame_size.unwrap_or_else(|| terminal::size().unwrap_or((0, 0))));
        self.use_terminal_events(move |event| {
            if let TerminalEvent::Resize(width, height) = event {
                size.set((width, height));
            }
        });
        // Events wake the component and provide a fallback outside a terminal
        // render loop. They are not the geometry authority for a frame: queued
        // resize events can lag behind the renderer's current size sample.
        frame_size.unwrap_or_else(|| size.get())
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use futures::stream::StreamExt;
    use macro_rules_attribute::apply;
    use smol_macros::test;

    #[component]
    fn MyComponent(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
        let mut system = hooks.use_context_mut::<SystemContext>();
        let (width, height) = hooks.use_terminal_size();

        if width == 100 && height == 40 {
            system.exit();
        }

        element! {
            Text(content: format!("{}x{}", width, height))
        }
    }

    #[apply(test!)]
    async fn test_use_terminal_size() {
        let actual = element!(MyComponent)
            .mock_terminal_render_loop(MockTerminalConfig::with_events(futures::stream::iter(
                vec![TerminalEvent::Resize(100, 40)],
            )))
            .collect::<Vec<_>>()
            .await;
        let canvas = actual.last().unwrap();
        assert_eq!(canvas.to_string(), "100x40\n");
        assert_eq!(canvas.width(), 100, "layout and hook share the same size");
    }

    #[test]
    fn frame_snapshot_wins_over_hook_state_without_waiting_for_resize_events() {
        let mut storage = Vec::new();
        for (index, dimensions) in [(240, 40), (100, 18), (240, 40)].into_iter().enumerate() {
            let mut snapshot = crate::context::TerminalSizeSnapshot(Some(dimensions));
            let stack = crate::ContextStack::root(&mut snapshot);
            let mut hooks = crate::Hooks::new(&mut storage, index == 0);
            assert_eq!(
                hooks.with_context_stack(&stack).use_terminal_size(),
                dimensions,
                "the hook's stored initial size must not override this frame"
            );
        }
    }

    #[test]
    fn sibling_size_hooks_read_one_snapshot_while_system_context_is_borrowed() {
        #[component]
        fn Report(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
            let _system = hooks.use_context_mut::<SystemContext>();
            let (columns, rows) = hooks.use_terminal_size();
            element!(Text(content: format!("{columns}x{rows}")))
        }

        let canvas = element! {
            ContextProvider(value: Context::owned(crate::context::TerminalSizeSnapshot(Some((73, 19))))) {
                View(flex_direction: FlexDirection::Column) {
                    Report
                    Report
                }
            }
        }.render(Some(73));
        assert_eq!(canvas.to_string(), "73x19\n73x19\n");
    }
}
