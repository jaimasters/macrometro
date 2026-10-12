//! Zone-level demand, reachability-driven growth and sprawl.
//! Spec: docs/features/growth/SPEC.md. Pure core in `core`; this file is the bevy plugin.
use bevy::prelude::*;

mod agency;
mod core;
pub use agency::*;
pub use core::*;

/// Growth state in the ECS (transit writes `Costs`; growth reads it).
#[derive(Resource, Default)]
pub struct Costs(pub Option<CostMatrix>);

#[derive(Resource)]
pub struct Growth(pub GrowthSim);

/// Latest per-zone accessibility, for economy/render readers.
#[derive(Resource, Default)]
pub struct Accessibility(pub Vec<f64>);

/// Public entry point for this feature.
pub struct GrowthPlugin;

impl Plugin for GrowthPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Costs>()
            .init_resource::<Accessibility>()
            .insert_resource(Growth(GrowthSim::new(Zones::default(), GrowthParams::default())))
            .add_systems(FixedUpdate, growth_system);
    }
}

fn growth_system(costs: Res<Costs>, mut g: ResMut<Growth>, mut acc: ResMut<Accessibility>) {
    let Some(c) = &costs.0 else { return };
    if c.n != g.0.zones.0.len() {
        return;
    }
    acc.0 = g.0.step(c).accessibility;
}

#[cfg(test)]
mod plugin_tests {
    use super::*;

    #[test]
    fn plugin_step_changes_zones() {
        let mut app = App::new();
        app.add_plugins(GrowthPlugin);
        app.world_mut().resource_mut::<Growth>().0.zones = Zones::new(vec![
            Zone { id: ZoneId(0), pop: 100.0, jobs: 50.0, capacity: 1000.0, crowding: 0.0 },
            Zone { id: ZoneId(1), pop: 10.0, jobs: 200.0, capacity: 1000.0, crowding: 0.0 },
        ]);
        app.world_mut().resource_mut::<Costs>().0 =
            Some(CostMatrix::from_fn(2, |i, j| if i == j { 0.0 } else { 5.0 }));
        let before = app.world().resource::<Growth>().0.state_hash();
        app.world_mut().run_schedule(FixedUpdate);
        assert_ne!(before, app.world().resource::<Growth>().0.state_hash());
        assert_eq!(app.world().resource::<Accessibility>().0.len(), 2);
    }
}
