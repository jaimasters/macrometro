//! Rail and road networks; lines with specialisation attributes. Supplies growth::Costs and per-zone transit shares.
//! Spec: docs/features/transit/SPEC.md. Pure core in `core`; this file is the bevy plugin.
use bevy::prelude::*;

mod core;
pub mod trains;
pub use core::*;

#[derive(Resource, Default)]
pub struct TransitNet(pub Network);

/// Graph node of zone i (ascending ZoneId order, same indexing as growth::Zones / CostMatrix).
#[derive(Resource, Default)]
pub struct ZoneNodes(pub Vec<NodeId>);

/// Agency-tagged commands applied at the start of the next FixedUpdate.
#[derive(Resource, Default)]
pub struct TransitCommands(pub Vec<TransitCommand>);

/// Commands refused last tick; never silent.
#[derive(Resource, Default)]
pub struct TransitErrors(pub Vec<(TransitCommand, TransitError)>);

/// Per-zone transit share, input for growth::RoadAgency::observe.
#[derive(Resource, Default)]
pub struct TransitShares(pub Vec<f64>);

#[derive(Resource)]
struct Dirty(bool);

/// Public entry point for this feature.
pub struct TransitPlugin;

impl Plugin for TransitPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<growth::Costs>()
            .init_resource::<TransitNet>()
            .init_resource::<ZoneNodes>()
            .init_resource::<TransitCommands>()
            .init_resource::<TransitErrors>()
            .init_resource::<TransitShares>()
            .insert_resource(Dirty(true))
            .add_systems(FixedUpdate, (apply_commands, publish).chain());
    }
}

fn apply_commands(mut net: ResMut<TransitNet>, mut q: ResMut<TransitCommands>, mut errs: ResMut<TransitErrors>, mut dirty: ResMut<Dirty>) {
    errs.0.clear();
    for c in std::mem::take(&mut q.0) {
        match net.0.apply(&c) {
            Ok(_) => dirty.0 = true,
            Err(e) => errs.0.push((c, e)),
        }
    }
}

fn publish(
    net: Res<TransitNet>,
    nodes: Res<ZoneNodes>,
    growth: Option<Res<growth::Growth>>,
    mut costs: ResMut<growth::Costs>,
    mut shares: ResMut<TransitShares>,
    mut dirty: ResMut<Dirty>,
) {
    if !dirty.0 || nodes.0.is_empty() || nodes.0.iter().any(|n| n.0 >= net.0.nodes) {
        return;
    }
    dirty.0 = false;
    costs.0 = Some(cost_matrix(&net.0, &nodes.0));
    let w: Vec<f64> = match growth.as_ref().filter(|g| g.0.zones.0.len() == nodes.0.len()) {
        Some(g) => g.0.zones.0.iter().map(|z| z.jobs).collect(),
        None => vec![1.0; nodes.0.len()],
    };
    shares.0 = transit_shares(&net.0, &nodes.0, &w);
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
