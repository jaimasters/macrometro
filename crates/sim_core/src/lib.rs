//! Fixed-step simulation clock on worker threads and snapshot handoff to render.
//! Spec: docs/features/sim_core/SPEC.md. Pure core in `core`; this file is the bevy plugin.
use bevy::prelude::*;

mod core;
pub use core::*;

/// Monotonic simulation tick counter published with every snapshot.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimTick(pub u64);

/// Render-side handle: send commands, read latest snapshot.
#[derive(Resource)]
pub struct SimLink {
    pub commands: CommandSender,
    pub snapshots: std::sync::Arc<Handoff<CoreSnapshot>>,
}

/// Owns the worker thread so it joins on app drop (non-wasm only).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
struct SimThreadGuard(#[allow(dead_code)] SimThread);

/// Public entry point for this feature.
pub struct SimCorePlugin;

impl Plugin for SimCorePlugin {
    fn build(&self, app: &mut App) {
        let (sim, commands, snapshots) = Sim::new(CoreSim::new(1));
        app.init_resource::<SimTick>().insert_resource(SimLink { commands, snapshots });
        #[cfg(not(target_arch = "wasm32"))]
        app.insert_resource(SimThreadGuard(SimThread::spawn(sim, SIM_HZ)));
        // ponytail: wasm time-slicing not wired until app owns a Sim + budget (D-6); drop sim there.
        #[cfg(target_arch = "wasm32")]
        drop(sim);
        app.add_systems(First, read_snapshot);
    }
}

fn read_snapshot(link: Res<SimLink>, mut tick: ResMut<SimTick>) {
    tick.0 = link.snapshots.latest().tick;
}

#[cfg(test)]
mod plugin_tests {
    use super::*;

    #[test]
    fn plugin_updates_simtick() {
        let mut app = App::new();
        app.add_plugins(SimCorePlugin);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.world().resource::<SimTick>().0 < 2 {
            assert!(std::time::Instant::now() < deadline, "tick never advanced");
            std::thread::sleep(std::time::Duration::from_millis(20));
            app.world_mut().run_schedule(First);
        }
    }
}
