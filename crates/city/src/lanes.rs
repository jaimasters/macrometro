//! R24 road lane classes. A class sets capacity, speed and width; upgrading an edge widens its corridor, so it razes
//! whatever the extra width sweeps at the same eminent-domain price as a track curve (`placer::eminent_domain`).
use crate::placer::{eminent_domain, Demolition};
use crate::roadgen::TownPlan;
use bevy::math::Vec2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LaneClass {
    /// One shared lane, slow, cars pass slowly.
    Street,
    TwoLaneLow,
    TwoLaneHigh,
    FourLane,
    SixLane,
    EightLane,
    TenLane,
}

pub const LANE_CLASSES: [LaneClass; 7] = [
    LaneClass::Street,
    LaneClass::TwoLaneLow,
    LaneClass::TwoLaneHigh,
    LaneClass::FourLane,
    LaneClass::SixLane,
    LaneClass::EightLane,
    LaneClass::TenLane,
];

/// Lane width in world units.
const LANE_W: f32 = 0.9;
/// Cost per world unit of road length per lane added.
pub const LANE_COST_PER_LEN: f64 = 1.2;

/// The class every generated road starts as.
pub fn default_class(c: crate::roadgen::RoadClass) -> LaneClass {
    if c == crate::roadgen::RoadClass::Arterial { LaneClass::TwoLaneHigh } else { LaneClass::TwoLaneLow }
}

impl LaneClass {
    pub fn name(self) -> &'static str {
        ["street", "2-lane", "2-lane fast", "4-lane", "6-lane freeway", "8-lane freeway", "10-lane freeway"][self as usize]
    }
    pub fn lanes(self) -> u32 {
        [1, 2, 2, 4, 6, 8, 10][self as usize]
    }
    /// Free-flow speed (world units / min).
    pub fn speed(self) -> f64 {
        [3.0, 4.0, 7.0, 9.0, 11.0, 12.0, 13.0][self as usize]
    }
    /// Vehicles per hour: ~ 150 per lane, a shared street lane carries less.
    pub fn capacity(self) -> f64 {
        [120.0, 300.0, 900.0, 1400.0, 2000.0, 2600.0, 3200.0][self as usize]
    }
    /// Corridor width (lanes plus a freeway median from 6 lanes up).
    pub fn width(self) -> f32 {
        self.lanes() as f32 * LANE_W + if self.lanes() >= 6 { 1.6 } else { 0.0 }
    }
    pub fn next(self) -> Option<LaneClass> {
        LANE_CLASSES.get(self as usize + 1).copied()
    }
}

/// Price of widening a road along `strip` (its centreline, length `len`) to the next class: extra lanes plus what the
/// extra half-width razes. None at the top class.
pub struct LaneUpgrade {
    pub to: LaneClass,
    pub demolition: Demolition,
    pub cost: f64,
}

pub fn upgrade(plan: &TownPlan, pop: &[f32], gone: &[bool], strip: &[Vec2], len: f32, from: LaneClass) -> Option<LaneUpgrade> {
    let to = from.next()?;
    // only the width beyond the current corridor counts: the old road's footprint is already clear
    let demolition = eminent_domain(plan, pop, gone, strip, to.width() / 2.0);
    let cleared = eminent_domain(plan, pop, gone, strip, from.width() / 2.0);
    let zones: Vec<usize> = demolition.zones.iter().copied().filter(|z| !cleared.zones.contains(z)).collect();
    let share = if demolition.zones.is_empty() { 0.0 } else { zones.len() as f64 / demolition.zones.len() as f64 };
    let d = Demolition { cost: demolition.cost * share, standing: demolition.standing * share, zones };
    let cost = d.cost + (to.lanes() - from.lanes()) as f64 * len as f64 * LANE_COST_PER_LEN;
    Some(LaneUpgrade { to, demolition: d, cost })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_rise_in_capacity_speed_and_width() {
        for w in LANE_CLASSES.windows(2) {
            assert!(w[1].capacity() > w[0].capacity() && w[1].speed() >= w[0].speed() && w[1].width() >= w[0].width());
        }
        assert_eq!(LaneClass::TenLane.next(), None);
        assert!(LaneClass::SixLane.width() - LaneClass::FourLane.width() > 2.0 * LANE_W, "freeway median adds width");
    }
}
