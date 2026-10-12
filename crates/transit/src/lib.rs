//! Rail and road networks; lines with specialisation attributes.
//! Spec: docs/features/transit/SPEC.md. Skeleton only: no gameplay yet.
use bevy::prelude::*;

/// Rail specialisation a line converges to. All rail is one interchangeable system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RailClass { Tram, LightRail, HeavyRail, HighSpeed, DoubleDecker }

/// Public entry point for this feature.
pub struct TransitPlugin;

impl Plugin for TransitPlugin {
    fn build(&self, _app: &mut App) {}
}
