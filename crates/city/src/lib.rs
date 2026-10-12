//! Procedural towns: road graph generation, road growth, traffic and mode choice. Pure Rust (no ECS).
//! Spec: docs/features/city/SPEC.md; design: docs/arch/city-gen.md.
pub mod junctions;
pub mod lanes;
pub mod organic;
pub mod placer;
pub mod roadbuild;
pub mod roadgen;
pub mod traffic;
pub mod trips;
pub mod transit_times;
pub use roadbuild::*;
pub use roadgen::*;
pub use traffic::*;
pub use trips::*;
pub use transit_times::*;
