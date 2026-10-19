use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

/// The continuously enabled interval of one mounted input owner.
#[derive(Default)]
pub(crate) struct InputContinuity {
    active_since: Option<u64>,
}

#[derive(Clone)]
struct FrameControl {
    continuity: Arc<Mutex<InputContinuity>>,
    enabled: Arc<AtomicBool>,
}

#[derive(Default)]
pub(crate) struct InputFrameState {
    rendering: u64,
    presented: Option<u64>,
    rendering_size: Option<(u16, u16)>,
    presented_size: Option<(u16, u16)>,
    observed: Option<InputObservation>,
    rendering_controls: Vec<FrameControl>,
    allow_unknown_size: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct InputObservation {
    size: Option<(u16, u16)>,
    revision: Option<u64>,
    deferred: bool,
}

impl InputFrameState {
    pub fn present(&mut self) {
        for control in self.rendering_controls.drain(..) {
            let mut continuity = control.continuity.lock().unwrap();
            if control.enabled.load(Ordering::Relaxed) {
                continuity.active_since.get_or_insert(self.rendering);
            } else {
                continuity.active_since = None;
            }
        }
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
            revision: (size.is_some() || self.allow_unknown_size)
                .then_some(self.presented)
                .flatten(),
            deferred: self.presented_size != size,
        }
    }

    /// Wait for one presentation, not for an obsolete size to return. The
    /// callback then checks the actual geometry and focus continuity, so rapid
    /// reflow cannot stall input behind a size which was never presented.
    pub fn pending(&self, observation: InputObservation) -> bool {
        observation.deferred
            && observation.revision.is_some()
            && observation.revision == self.presented
    }

    pub fn observe(&mut self, observation: InputObservation) {
        self.observed = Some(observation);
    }
}

/// A component's input eligibility relative to its successfully presented frame.
///
/// Use this inside keyboard/paste callbacks before confirming an action. A
/// resize notification can follow the corresponding input on Unix; an old
/// callback must not authorize that input using an obsolete viewport.
/// Geometry-crossing events wait for reflow and require continuous enablement
/// across both presented frames. The caller owns focus/reviewability and
/// declares it with [`Self::set_enabled`] while rendering.
#[derive(Clone, Default)]
pub struct InputFrame {
    state: Option<Arc<Mutex<InputFrameState>>>,
    revision: u64,
    size: Option<(u16, u16)>,
    control: Option<FrameControl>,
}

impl InputFrame {
    pub(crate) fn begin(state: Arc<Mutex<InputFrameState>>, size: Option<(u16, u16)>) -> Self {
        let revision = {
            let mut state = state.lock().unwrap();
            state.rendering = state.rendering.wrapping_add(1);
            state.rendering_size = size;
            state.rendering_controls.clear();
            state.rendering
        };
        Self {
            state: Some(state),
            revision,
            size,
            control: None,
        }
    }

    pub(crate) fn for_control(&self, continuity: Arc<Mutex<InputContinuity>>) -> Self {
        let control = FrameControl {
            continuity,
            enabled: Arc::new(AtomicBool::new(true)),
        };
        if let Some(state) = &self.state {
            state
                .lock()
                .unwrap()
                .rendering_controls
                .push(control.clone());
        }
        Self {
            control: Some(control),
            ..self.clone()
        }
    }

    /// Declare whether this owner can be operated in the frame being rendered.
    /// Call during each render, before presentation; a skipped/failed
    /// presentation does not change the continuously enabled interval.
    ///
    /// TextInput does this automatically from `has_focus`. Custom controls
    /// supply their own focus/reviewability, without recomputing geometry here.
    /// Cancellation may remain outside this guard.
    pub fn set_enabled(&self, enabled: bool) {
        if let Some(control) = &self.control {
            control.enabled.store(enabled, Ordering::Relaxed);
        }
    }

    /// Whether this callback belongs to the presented frame and its incoming
    /// event was observed at that frame's dimensions.
    ///
    /// Call only while handling a keyboard or paste event. Non-terminal/static
    /// rendering has no input geometry and does not impose an additional gate.
    pub fn is_current(&self) -> bool {
        if self
            .control
            .as_ref()
            .is_some_and(|control| !control.enabled.load(Ordering::Relaxed))
        {
            return false;
        }
        self.state.as_ref().is_none_or(|state| {
            let state = state.lock().unwrap();
            state.presented == Some(self.revision)
                && state.observed.is_some_and(|observation| {
                    observation.size == self.size
                        && observation.revision.is_some_and(|origin| {
                            origin == self.revision
                                || (observation.deferred
                                    && self.control.as_ref().is_some_and(|control| {
                                        control
                                            .continuity
                                            .lock()
                                            .unwrap()
                                            .active_since
                                            .is_some_and(|since| origin >= since)
                                    }))
                        })
                })
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
        let state = Arc::<Mutex<InputFrameState>>::default();
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
        let state = Arc::<Mutex<InputFrameState>>::default();
        let frame = InputFrame::begin(state.clone(), LARGE);
        state.lock().unwrap().present();
        observe(&state, SMALL);
        assert!(!frame.is_current());
    }

    #[test]
    fn growth_does_not_authorize_input_collected_in_the_small_frame() {
        let state = Arc::<Mutex<InputFrameState>>::default();
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
        let state = Arc::<Mutex<InputFrameState>>::default();
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
        let state = Arc::<Mutex<InputFrameState>>::default();
        let old = InputFrame::begin(state.clone(), LARGE);
        state.lock().unwrap().present();
        let unpresented = InputFrame::begin(state.clone(), SMALL);
        observe(&state, LARGE);
        assert!(old.is_current());
        assert!(!unpresented.is_current());
    }

    #[test]
    fn real_terminal_with_unknown_geometry_fails_closed() {
        let state = Arc::<Mutex<InputFrameState>>::default();
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

    #[test]
    fn continuously_enabled_resize_input_waits_then_survives() {
        let state = Arc::<Mutex<InputFrameState>>::default();
        let continuity = Arc::<Mutex<InputContinuity>>::default();
        let old = InputFrame::begin(state.clone(), LARGE).for_control(continuity.clone());
        state.lock().unwrap().present();
        let size = Some((94, 30));
        let observation = state.lock().unwrap().sample(size);
        assert!(state.lock().unwrap().pending(observation));
        state.lock().unwrap().observe(observation);
        assert!(!old.is_current());
        let new = InputFrame::begin(state.clone(), size).for_control(continuity);
        state.lock().unwrap().present();
        assert!(!state.lock().unwrap().pending(observation));
        assert!(new.is_current());
        assert!(!old.is_current());
    }

    #[test]
    fn deferred_input_cannot_enter_a_disabled_frame() {
        let state = Arc::<Mutex<InputFrameState>>::default();
        let continuity = Arc::<Mutex<InputContinuity>>::default();
        InputFrame::begin(state.clone(), LARGE).for_control(continuity.clone());
        state.lock().unwrap().present();
        let observation = state.lock().unwrap().sample(SMALL);
        let small = InputFrame::begin(state.clone(), SMALL).for_control(continuity);
        small.set_enabled(false);
        state.lock().unwrap().present();
        state.lock().unwrap().observe(observation);
        assert!(!small.is_current());
    }

    #[test]
    fn growth_cannot_authorize_input_originating_in_a_disabled_frame() {
        let state = Arc::<Mutex<InputFrameState>>::default();
        let continuity = Arc::<Mutex<InputContinuity>>::default();
        let small = InputFrame::begin(state.clone(), SMALL).for_control(continuity.clone());
        small.set_enabled(false);
        state.lock().unwrap().present();
        let observation = state.lock().unwrap().sample(LARGE);
        let large = InputFrame::begin(state.clone(), LARGE).for_control(continuity);
        state.lock().unwrap().present();
        state.lock().unwrap().observe(observation);
        assert!(!large.is_current());
        observe(&state, LARGE);
        assert!(large.is_current());
    }

    #[test]
    fn skipped_hidden_render_does_not_break_presented_focus_continuity() {
        let state = Arc::<Mutex<InputFrameState>>::default();
        let continuity = Arc::<Mutex<InputContinuity>>::default();
        InputFrame::begin(state.clone(), LARGE).for_control(continuity.clone());
        state.lock().unwrap().present();
        let size = Some((94, 30));
        let observation = state.lock().unwrap().sample(size);
        let hidden = InputFrame::begin(state.clone(), SMALL).for_control(continuity.clone());
        hidden.set_enabled(false);
        let visible = InputFrame::begin(state.clone(), size).for_control(continuity);
        state.lock().unwrap().present();
        state.lock().unwrap().observe(observation);
        assert!(visible.is_current());
    }

    #[test]
    fn presented_hidden_frame_breaks_continuity() {
        let state = Arc::<Mutex<InputFrameState>>::default();
        let continuity = Arc::<Mutex<InputContinuity>>::default();
        InputFrame::begin(state.clone(), LARGE).for_control(continuity.clone());
        state.lock().unwrap().present();
        let size = Some((94, 30));
        let observation = state.lock().unwrap().sample(size);
        let hidden = InputFrame::begin(state.clone(), SMALL).for_control(continuity.clone());
        hidden.set_enabled(false);
        state.lock().unwrap().present();
        let visible = InputFrame::begin(state.clone(), size).for_control(continuity);
        state.lock().unwrap().present();
        state.lock().unwrap().observe(observation);
        assert!(!visible.is_current());
    }

    #[test]
    fn rapid_reflow_does_not_wait_forever_for_an_unpresented_size() {
        let state = Arc::<Mutex<InputFrameState>>::default();
        let continuity = Arc::<Mutex<InputContinuity>>::default();
        InputFrame::begin(state.clone(), LARGE).for_control(continuity.clone());
        state.lock().unwrap().present();
        let observation = state.lock().unwrap().sample(SMALL);
        let new = InputFrame::begin(state.clone(), LARGE).for_control(continuity);
        state.lock().unwrap().present();
        assert!(!state.lock().unwrap().pending(observation));
        state.lock().unwrap().observe(observation);
        assert!(!new.is_current());
        observe(&state, LARGE);
        assert!(new.is_current());
    }
}
