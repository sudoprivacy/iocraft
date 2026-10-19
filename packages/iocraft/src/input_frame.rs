use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct InputFrameState {
    rendering: u64,
    presented: Option<u64>,
    rendering_size: Option<(u16, u16)>,
    presented_size: Option<(u16, u16)>,
    observed_size: Option<(u16, u16)>,
    observed_eligible: bool,
    allow_unknown_size: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct InputObservation {
    size: Option<(u16, u16)>,
    eligible: bool,
}

impl InputFrameState {
    pub fn present(&mut self) {
        self.presented = Some(self.rendering);
        self.presented_size = self.rendering_size;
    }

    pub fn mock() -> Self {
        Self {
            allow_unknown_size: true,
            ..Self::default()
        }
    }

    pub fn sample(&self, size: Option<(u16, u16)>) -> InputObservation {
        InputObservation {
            size,
            eligible: self.presented.is_some()
                && self.presented_size == size
                && (size.is_some() || self.allow_unknown_size),
        }
    }

    pub fn observe(&mut self, observation: InputObservation) {
        self.observed_size = observation.size;
        self.observed_eligible = observation.eligible;
    }
}

/// A component's input eligibility relative to its successfully presented frame.
///
/// Use this inside keyboard/paste callbacks before confirming an action. A
/// resize notification can follow the corresponding input on Unix; an old
/// callback must not authorize that input using an obsolete viewport.
/// This checks presentation and geometry, not application-specific focus.
#[derive(Clone, Default)]
pub struct InputFrame {
    state: Option<Arc<Mutex<InputFrameState>>>,
    revision: u64,
    size: Option<(u16, u16)>,
}

impl InputFrame {
    pub(crate) fn begin(state: Arc<Mutex<InputFrameState>>, size: Option<(u16, u16)>) -> Self {
        let revision = {
            let mut state = state.lock().unwrap();
            state.rendering = state.rendering.wrapping_add(1);
            state.rendering_size = size;
            state.rendering
        };
        Self {
            state: Some(state),
            revision,
            size,
        }
    }

    /// Whether this callback belongs to the presented frame and its incoming
    /// event was observed at that frame's dimensions.
    ///
    /// Call only while handling a keyboard or paste event. Non-terminal/static
    /// rendering has no input geometry and does not impose an additional gate.
    pub fn is_current(&self) -> bool {
        self.state.as_ref().is_none_or(|state| {
            let state = state.lock().unwrap();
            state.observed_eligible
                && state.presented == Some(self.revision)
                && state.observed_size == self.size
        })
    }
}
