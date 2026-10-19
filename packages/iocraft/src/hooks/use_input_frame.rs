use crate::{Hooks, InputFrame};

mod private {
    pub trait Sealed {}
    impl Sealed for crate::Hooks<'_, '_> {}
}

/// Access to the current component's presented input frame.
pub trait UseInputFrame: private::Sealed {
    /// Returns a frame guard for use inside keyboard/paste event callbacks.
    fn use_input_frame(&mut self) -> InputFrame;
}

impl UseInputFrame for Hooks<'_, '_> {
    fn use_input_frame(&mut self) -> InputFrame {
        self.context_stack
            .and_then(|stack| stack.get_context::<InputFrame>())
            .map(|frame| frame.clone())
            .unwrap_or_default()
    }
}
