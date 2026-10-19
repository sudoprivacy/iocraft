use crate::{hooks::UseMemo, input_frame::InputContinuity, Hooks, InputFrame};
use std::sync::{Arc, Mutex};

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
        let frame = self
            .context_stack
            .and_then(|stack| stack.get_context::<InputFrame>())
            .map(|frame| frame.clone())
            .unwrap_or_default();
        let continuity = self.use_memo(Arc::<Mutex<InputContinuity>>::default, ());
        frame.for_control(continuity)
    }
}
