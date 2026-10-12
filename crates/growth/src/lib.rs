//! Zone-level demand, reachability-driven growth and sprawl.
//! Spec: docs/features/growth/SPEC.md. Skeleton only: no gameplay yet.
use bevy::prelude::*;

/// Coarse zone identifier for aggregated (out-of-focus) simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ZoneId(pub u32);

/// Public entry point for this feature.
pub struct GrowthPlugin;

impl Plugin for GrowthPlugin {
    fn build(&self, _app: &mut App) {}
}
