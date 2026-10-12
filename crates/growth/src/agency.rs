//! City road agency (D-7): underserved zones -> RoadPlan one cycle ahead + road_reserve. Pure, deterministic.
use crate::core::{ZoneId, Zones};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AgencyParams {
    /// Transit share below this counts as underserved.
    pub s_min: f64,
    /// Consecutive underserved ticks before the city plans a road.
    pub t_ticks: u32,
    /// Ticks between emitting a plan (UI ghost) and building it ("one cycle ahead").
    pub lead_ticks: u64,
    /// Reserve per resident of the zone for one plan.
    pub cost_per_pop: f64,
}

impl Default for AgencyParams {
    fn default() -> Self {
        Self { s_min: 0.2, t_ticks: 100, lead_ticks: 100, cost_per_pop: 1.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadPlan {
    pub zone: ZoneId,
    pub emitted_tick: u64,
    /// Tick at which the city builds it (economy consumes `cost` from road_reserve then).
    pub build_tick: u64,
    pub cost: f64,
}

#[derive(Debug, Default, PartialEq)]
pub struct AgencyOutput {
    /// Newly emitted plans this tick (UI ghosts).
    pub planned: Vec<RoadPlan>,
    /// Plans whose build_tick was reached this tick.
    pub due: Vec<RoadPlan>,
    /// Sum of cost of all pending (planned, not yet due) plans: demand figure for economy.
    pub road_reserve: f64,
}

#[derive(Debug, Clone, Default)]
pub struct RoadAgency {
    pub params: AgencyParams,
    /// Consecutive underserved ticks per zone index.
    streak: Vec<u32>,
    pending: Vec<RoadPlan>,
}

impl RoadAgency {
    pub fn new(params: AgencyParams) -> Self {
        Self { params, streak: Vec::new(), pending: Vec::new() }
    }

    pub fn pending(&self) -> &[RoadPlan] {
        &self.pending
    }

    /// One tick. `shares[i]` = transit share of zones.0[i] (from transit). Zones in ascending id order => plans in id order.
    pub fn observe(&mut self, tick: u64, zones: &Zones, shares: &[f64]) -> AgencyOutput {
        assert_eq!(zones.0.len(), shares.len());
        self.streak.resize(zones.0.len(), 0);
        let mut out = AgencyOutput::default();
        for (i, z) in zones.0.iter().enumerate() {
            let already = self.pending.iter().any(|p| p.zone == z.id);
            if shares[i] < self.params.s_min && !already {
                self.streak[i] += 1;
            } else {
                self.streak[i] = 0;
            }
            if self.streak[i] >= self.params.t_ticks {
                self.streak[i] = 0;
                let p = RoadPlan {
                    zone: z.id,
                    emitted_tick: tick,
                    build_tick: tick + self.params.lead_ticks,
                    cost: z.pop * self.params.cost_per_pop,
                };
                self.pending.push(p);
                out.planned.push(p);
            }
        }
        let (due, keep): (Vec<_>, Vec<_>) = self.pending.iter().partition(|p| p.build_tick <= tick);
        out.due = due.into_iter().copied().collect();
        self.pending = keep.into_iter().copied().collect();
        out.road_reserve = self.pending.iter().map(|p| p.cost).sum();
        out
    }
}

#[cfg(test)]
#[path = "agency_tests.rs"]
mod tests;
