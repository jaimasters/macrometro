//! Pure tool logic: command types, wire encoding, ray picking, click state machine, ghost geometry.
//! No ECS here except the `Message` derives; everything is unit-tested below.
use bevy::prelude::*;

/// Acting agency (D-8). Every player command carries one so the sim stays input-driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct AgencyId(pub u16);

/// Which tool the player has selected.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveTool {
    #[default]
    None,
    PlaceStation,
    DrawLine,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolAction {
    PlaceStation { cell: IVec2 },
    /// Station cells in travel order (>= 2).
    DrawLine { cells: Vec<IVec2> },
}

/// A player command, always tagged with the acting agency (D-8).
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct ToolCommand {
    pub agency: AgencyId,
    pub action: ToolAction,
}

/// Planned (not yet built) road segments to draw as faint ghosts (D-7).
/// ponytail: stub until `growth` publishes its own RoadPlan; swap the type then.
#[derive(Message, Debug, Clone, PartialEq, Default)]
pub struct RoadPlan {
    pub segments: Vec<(Vec2, Vec2)>,
}

/// Wire kind for `sim_core::Command::Custom` carrying a PlaceStation.
pub const KIND_PLACE_STATION: u32 = 0x5701;

const I24: i64 = 1 << 23;

impl ToolCommand {
    /// Pack into `Command::Custom(kind, arg)`. Only PlaceStation is encodable today
    /// (sim_core::Command is Copy with no payload room; see report for the needed change).
    pub fn encode(&self) -> Option<(u32, i64)> {
        let ToolAction::PlaceStation { cell } = &self.action else { return None };
        let (x, z) = (cell.x as i64, cell.y as i64);
        if !(-I24..I24).contains(&x) || !(-I24..I24).contains(&z) {
            return None;
        }
        let arg = ((self.agency.0 as i64) << 48) | ((x & 0xFF_FFFF) << 24) | (z & 0xFF_FFFF);
        Some((KIND_PLACE_STATION, arg))
    }

    pub fn decode(kind: u32, arg: i64) -> Option<Self> {
        if kind != KIND_PLACE_STATION {
            return None;
        }
        let sext = |v: i64| (v << 40) >> 40; // sign-extend 24 bits
        Some(Self {
            agency: AgencyId((arg >> 48) as u16),
            action: ToolAction::PlaceStation { cell: IVec2::new(sext(arg >> 24) as i32, sext(arg) as i32) },
        })
    }
}

/// March a ray until it dips below `ground(x,z)`, then bisect. Returns the world (x,z) hit.
pub fn ray_terrain(origin: Vec3, dir: Vec3, ground: &dyn Fn(f32, f32) -> f32) -> Option<Vec2> {
    const STEP: f32 = 1.0;
    const MAX_T: f32 = 4000.0;
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let below = |t: f32| {
        let p = origin + dir * t;
        p.y <= ground(p.x, p.z)
    };
    let mut t = 0.0;
    while t < MAX_T {
        if below(t + STEP) {
            let (mut lo, mut hi) = (t, t + STEP);
            for _ in 0..12 {
                let mid = 0.5 * (lo + hi);
                if below(mid) { hi = mid } else { lo = mid }
            }
            let p = origin + dir * hi;
            return Some(Vec2::new(p.x, p.z));
        }
        t += STEP;
    }
    None
}

pub fn snap(p: Vec2) -> IVec2 {
    IVec2::new(p.x.round() as i32, p.y.round() as i32)
}

/// Pending DrawLine stations.
#[derive(Resource, Debug, Clone, Default)]
pub struct LineDraft(pub Vec<IVec2>);

/// A ground click with the active tool. PlaceStation emits at once; DrawLine accumulates.
pub fn click_action(tool: ActiveTool, agency: AgencyId, draft: &mut LineDraft, cell: IVec2) -> Option<ToolCommand> {
    match tool {
        ActiveTool::None => None,
        ActiveTool::PlaceStation => Some(ToolCommand { agency, action: ToolAction::PlaceStation { cell } }),
        ActiveTool::DrawLine => {
            if draft.0.last() != Some(&cell) {
                draft.0.push(cell);
            }
            None
        }
    }
}

/// Enter pressed: emit the drafted line if it has >= 2 stations, clearing the draft either way.
pub fn finish_line(agency: AgencyId, draft: &mut LineDraft) -> Option<ToolCommand> {
    let cells = std::mem::take(&mut draft.0);
    (cells.len() >= 2).then(|| ToolCommand { agency, action: ToolAction::DrawLine { cells } })
}

/// Transform for a unit cuboid (1x1x1) laid along segment a->b on the ground: scale = (length, thickness, width).
pub fn segment_transform(a: Vec2, b: Vec2, y: f32, width: f32, thickness: f32) -> Transform {
    let d = b - a;
    let mid = (a + b) * 0.5;
    Transform {
        translation: Vec3::new(mid.x, y, mid.y),
        // Cuboid X axis -> segment direction in the XZ plane (rotation about +Y; z maps to -sin).
        rotation: Quat::from_rotation_y(-d.y.atan2(d.x)),
        scale: Vec3::new(d.length().max(0.01), thickness, width),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_roundtrip_incl_negatives() {
        for cell in [IVec2::new(5, 9), IVec2::new(-300, 77), IVec2::new(-8_000_000, 8_000_000), IVec2::ZERO] {
            let c = ToolCommand { agency: AgencyId(3), action: ToolAction::PlaceStation { cell } };
            let (k, a) = c.encode().unwrap();
            assert_eq!(ToolCommand::decode(k, a), Some(c));
        }
        let line = ToolCommand { agency: AgencyId(1), action: ToolAction::DrawLine { cells: vec![] } };
        assert!(line.encode().is_none());
        assert!(ToolCommand::decode(1, 0).is_none());
    }

    #[test]
    fn ray_hits_flat_and_ramp() {
        let o = Vec3::new(10.0, 50.0, 10.0);
        let d = Vec3::new(1.0, -1.0, 0.0);
        let flat = ray_terrain(o, d, &|_, _| 0.0).unwrap();
        assert!((flat - Vec2::new(60.0, 10.0)).length() < 0.05, "{flat}");
        // Ground rises with x: y = 0.5x. Ray y = 50-(x-10). Hit where 60-x = 0.5x -> x = 40.
        let ramp = ray_terrain(o, d, &|x, _| 0.5 * x).unwrap();
        assert!((ramp.x - 40.0).abs() < 0.05, "{ramp}");
        assert!(ray_terrain(o, Vec3::Y, &|_, _| 0.0).is_none());
    }

    #[test]
    fn click_state_machine() {
        let ag = AgencyId(2);
        let mut d = LineDraft::default();
        assert!(click_action(ActiveTool::None, ag, &mut d, IVec2::ONE).is_none());
        let c = click_action(ActiveTool::PlaceStation, ag, &mut d, IVec2::new(4, 4)).unwrap();
        assert_eq!(c.agency, ag);
        assert!(click_action(ActiveTool::DrawLine, ag, &mut d, IVec2::new(0, 0)).is_none());
        assert!(finish_line(ag, &mut d).is_none()); // one point: no line, draft cleared
        assert!(d.0.is_empty());
        for p in [IVec2::new(0, 0), IVec2::new(0, 0), IVec2::new(9, 0)] {
            click_action(ActiveTool::DrawLine, ag, &mut d, p);
        }
        let line = finish_line(ag, &mut d).unwrap();
        assert_eq!(line.action, ToolAction::DrawLine { cells: vec![IVec2::new(0, 0), IVec2::new(9, 0)] });
    }

    #[test]
    fn segment_transform_axes() {
        let t = segment_transform(Vec2::new(0.0, 0.0), Vec2::new(0.0, 10.0), 1.0, 2.0, 0.5);
        assert!((t.scale.x - 10.0).abs() < 1e-5 && t.translation == Vec3::new(0.0, 1.0, 5.0));
        // Local +X must point along +Z (segment direction).
        assert!((t.rotation * Vec3::X - Vec3::Z).length() < 1e-5);
    }
}
