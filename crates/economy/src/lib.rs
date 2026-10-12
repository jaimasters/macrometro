//! Budgets, soft conditions, political standing, land cost; agencies are entities keyed by AgencyId.
//! Spec: docs/features/economy/SPEC.md. Pure core in `core`; this file is the bevy plugin.
use bevy::prelude::*;

mod core;
pub use core::*;

/// A player/agency command (D-8): always carries the issuing AgencyId.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpendCommand {
    pub agency: AgencyId,
    pub category: Category,
    pub amount: f64,
}

#[derive(Resource)]
pub struct Economy(pub EconomyState);

/// Latest scorecards (written by the metrics producer) and car share (from growth/transit).
#[derive(Resource, Default)]
pub struct Scorecards(pub Vec<(AgencyId, Scorecard)>);

#[derive(Resource, Default)]
pub struct CarShare(pub f64);

/// Cost of pending city road plans (growth::AgencyOutput::road_reserve); competes with agency budgets.
#[derive(Resource, Default)]
pub struct RoadDemand(pub f64);

/// Budget issued for the current period.
#[derive(Resource, Default)]
pub struct CurrentBudget(pub Option<Budget>);

#[derive(Resource, Default)]
pub struct SpendQueue(pub Vec<SpendCommand>);

/// Commands refused last tick (unknown agency / bad amount); never silent.
#[derive(Resource, Default)]
pub struct SpendErrors(pub Vec<(SpendCommand, EconError)>);

#[derive(Resource)]
pub struct PeriodClock {
    pub ticks: u32,
    pub period: u32,
}

/// Public entry point for this feature.
pub struct EconomyPlugin;

impl Plugin for EconomyPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Economy(EconomyState::new(EconomyParams::default())))
            .insert_resource(PeriodClock { ticks: 0, period: 100 })
            .init_resource::<Scorecards>()
            .init_resource::<CarShare>()
            .init_resource::<RoadDemand>()
            .init_resource::<CurrentBudget>()
            .init_resource::<SpendQueue>()
            .init_resource::<SpendErrors>()
            .add_systems(FixedUpdate, (spend_system, period_system).chain());
    }
}

fn spend_system(mut eco: ResMut<Economy>, mut q: ResMut<SpendQueue>, mut errs: ResMut<SpendErrors>) {
    errs.0.clear();
    for c in std::mem::take(&mut q.0) {
        if let Err(e) = record_spend(&mut eco.0, c.agency, c.category, c.amount) {
            errs.0.push((c, e));
        }
    }
}

fn period_system(
    mut eco: ResMut<Economy>,
    cards: Res<Scorecards>,
    car: Res<CarShare>,
    road: Res<RoadDemand>,
    mut clock: ResMut<PeriodClock>,
    mut budget: ResMut<CurrentBudget>,
) {
    clock.ticks += 1;
    if clock.period == 0 || clock.ticks % clock.period != 0 {
        return;
    }
    if let Some(issued) = &budget.0 {
        eco.0 = close_period(&eco.0, issued);
    }
    budget.0 = Some(next_budget(&eco.0, &cards.0, car.0, road.0));
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
