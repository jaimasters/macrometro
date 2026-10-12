//! Fixed-step simulation clock on worker threads and snapshot handoff to render.
//! Spec: docs/features/sim_core/SPEC.md. Skeleton only: no gameplay yet.
use bevy::prelude::*;

/// Fixed sim tick rate (Hz). Decoupled from render; see docs/arch/sim-render-decoupling.md.
pub const SIM_HZ: u32 = 10;

/// Monotonic simulation tick counter published with every snapshot.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimTick(pub u64);

/// Public entry point for this feature.
pub struct SimCorePlugin;

impl Plugin for SimCorePlugin {
    fn build(&self, _app: &mut App) {}
}
