//! Money, land value, greenfield vs compulsory-acquisition costs.
//! Spec: docs/features/economy/SPEC.md. Skeleton only: no gameplay yet.
use bevy::prelude::*;

/// Treasury balance in whole currency units.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Treasury(pub i64);

/// Public entry point for this feature.
pub struct EconomyPlugin;

impl Plugin for EconomyPlugin {
    fn build(&self, _app: &mut App) {}
}
