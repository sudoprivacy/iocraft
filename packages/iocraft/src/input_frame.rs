use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct InputFrameState {
    rendering: u64,
    presented: Option<u64>,
    rendering_size: Option<(u16, u16)>,
    presented_size: Option<(u16, u16)>,
    observed_size: Option<(u16, u16)>,
    observed_revision: Option<u64>,
    allow_unknown_size: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct InputObservation {
    size: Option<(u16, u16)>,
    revision: Option<u64>,
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
            revision: (self.presented_size == size && (size.is_some() || self.allow_unknown_size))
                .then_some(self.presented)
                .flatten(),
        }
    }

    pub fn observe(&mut self, observation: InputObservation) {
        self.observed_size = observation.size;
        self.observed_revision = observation.revision;
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
            state.observed_revision == Some(self.revision)
                && state.presented == Some(self.revision)
                && state.observed_size == self.size
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LARGE: Option<(u16, u16)> = Some((100, 30));
    const SMALL: Option<(u16, u16)> = Some((12, 8));

    fn observe(state: &Arc<Mutex<InputFrameState>>, size: Option<(u16, u16)>) {
        let mut state = state.lock().unwrap();
        let observation = state.sample(size);
        state.observe(observation);
    }

    #[test]
    fn input_requires_successful_presentation() {
        let state = Arc::default();
        let frame = InputFrame::begin(state.clone(), LARGE);
        observe(&state, LARGE);
        assert!(!frame.is_current());
        state.lock().unwrap().present();
        // Presentation cannot authorize an observation collected before it.
        assert!(!frame.is_current());
        observe(&state, LARGE);
        assert!(frame.is_current());
    }

    #[test]
    fn shrink_before_resize_notification_rejects_old_frame_input() {
        let state = Arc::default();
        let frame = InputFrame::begin(state.clone(), LARGE);
        state.lock().unwrap().present();
        observe(&state, SMALL);
        assert!(!frame.is_current());
    }

    #[test]
    fn growth_does_not_authorize_input_collected_in_the_small_frame() {
        let state = Arc::default();
        let small = InputFrame::begin(state.clone(), SMALL);
        state.lock().unwrap().present();
        let observation = state.lock().unwrap().sample(LARGE);
        let large = InputFrame::begin(state.clone(), LARGE);
        state.lock().unwrap().present();
        state.lock().unwrap().observe(observation);
        assert!(!small.is_current());
        assert!(!large.is_current());
        observe(&state, LARGE);
        assert!(large.is_current());
    }

    #[test]
    fn same_size_redraw_cannot_launder_an_older_observation() {
        let state = Arc::default();
        let old = InputFrame::begin(state.clone(), LARGE);
        state.lock().unwrap().present();
        let observation = state.lock().unwrap().sample(LARGE);
        let new = InputFrame::begin(state.clone(), LARGE);
        state.lock().unwrap().present();
        state.lock().unwrap().observe(observation);
        assert!(!old.is_current());
        assert!(!new.is_current());
        observe(&state, LARGE);
        assert!(!old.is_current());
        assert!(new.is_current());
    }

    #[test]
    fn unpresented_render_does_not_replace_the_presented_frame() {
        let state = Arc::default();
        let old = InputFrame::begin(state.clone(), LARGE);
        state.lock().unwrap().present();
        let unpresented = InputFrame::begin(state.clone(), SMALL);
        observe(&state, LARGE);
        assert!(old.is_current());
        assert!(!unpresented.is_current());
    }

    #[test]
    fn real_terminal_with_unknown_geometry_fails_closed() {
        let state = Arc::default();
        let frame = InputFrame::begin(state.clone(), None);
        state.lock().unwrap().present();
        observe(&state, None);
        assert!(!frame.is_current());
    }

    #[test]
    fn dimensionless_mock_and_static_rendering_remain_compatible() {
        let state = Arc::new(Mutex::new(InputFrameState::mock()));
        let frame = InputFrame::begin(state.clone(), None);
        state.lock().unwrap().present();
        observe(&state, None);
        assert!(frame.is_current());
        assert!(InputFrame::default().is_current());
    }
}
