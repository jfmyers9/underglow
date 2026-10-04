use crate::effects::EffectKind;
use crate::render::{Frame, RenderContext};
use crate::signals::SignalProgram;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug)]
pub struct StaticEffectSignal {
    effect: EffectKind,
}

impl StaticEffectSignal {
    pub fn new(effect: EffectKind) -> Self {
        Self { effect }
    }
}

impl SignalProgram for StaticEffectSignal {
    fn tick(&mut self, _interrupted: &AtomicBool) -> crate::signals::ProgramResult {
        Ok(())
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        self.effect.render(ctx)
    }

    fn finished(&self) -> bool {
        false
    }

    fn shutdown(&mut self, _interrupted: bool) -> crate::signals::ProgramResult {
        Ok(())
    }
}
