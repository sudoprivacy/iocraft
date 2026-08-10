use crate::{hook::Hook, render::ComponentUpdater, render::MeasureFunc};

/// Hook that sets a measure function on the component. The measure
/// function is called by taffy during layout to determine the component's
/// content size given the available space.
///
/// This allows `#[component]` functions to participate in the
/// measure-during-layout cycle, which is necessary for content-dependent
/// sizing (e.g. text wrapping that determines height from width).
pub trait UseMeasure {
    fn use_measure_func(&mut self, f: MeasureFunc);
}

impl<'a> UseMeasure for crate::Hooks<'a, '_> {
    fn use_measure_func(&mut self, f: MeasureFunc) {
        let hook = self.use_hook(|| UseMeasureImpl { func: None });
        hook.func = Some(f);
    }
}

struct UseMeasureImpl {
    func: Option<MeasureFunc>,
}

impl Hook for UseMeasureImpl {
    fn post_component_update(&mut self, updater: &mut ComponentUpdater) {
        if let Some(f) = self.func.take() {
            updater.set_measure_func(f);
        }
    }
}
