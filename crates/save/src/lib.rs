//! Versioned world save format, loader and seeded scenario generator.
//! Spec: docs/features/save/SPEC.md.
use bevy::prelude::*;

pub mod format;
pub mod loader;
pub mod scenario;

pub use format::*;
pub use loader::{from_ron, load_with, to_ron, LoadError, Migration, MIGRATIONS};

pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, _app: &mut App) {}
}
