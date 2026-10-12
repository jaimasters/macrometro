//! Monitor-locked frame pacing with prioritised, deferrable render work.
//! Spec: docs/features/render_budget/SPEC.md. Skeleton only: no gameplay yet.
use bevy::prelude::*;

/// Render work priority; lower value is drawn first and never deferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority { Critical, High, Normal, Deferrable }

/// Public entry point for this feature.
pub struct RenderBudgetPlugin;

impl Plugin for RenderBudgetPlugin {
    fn build(&self, _app: &mut App) {}
}
