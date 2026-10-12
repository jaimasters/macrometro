//! Pure tool logic: commands, ray picking, snapping, line-drag, octilinear routing, ghost geometry.
//! No ECS here except the `Message`/`Resource` derives; unit-tested below.
use bevy::prelude::*;

/// Acting agency (D-8): the shared sim_core type. Every player command carries one.
pub use sim_core::AgencyId;

/// Which tool the player has selected. `Pan` = drag the map.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveTool {
    #[default]
    Pan,
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

/// Faint planned-road segments (world x,z pairs) to draw as ghosts (D-7). Each message is the FULL
/// current set: the renderer replaces what it drew. App converts growth::RoadPlan (zone-level, no
/// geometry) into segments.
#[derive(Message, Debug, Clone, PartialEq, Default)]
pub struct GhostRoads {
    pub segments: Vec<(Vec2, Vec2)>,
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

/// Station grid pitch in world units; keeps 45/90 degree lines tidy.
pub const GRID: f32 = 4.0;
/// A press this close to an existing station picks that station.
pub const PICK_RADIUS: f32 = 7.0;

pub fn snap(p: Vec2) -> IVec2 {
    (p / GRID).round().as_ivec2() * GRID as i32
}

/// Nearest existing station within `PICK_RADIUS` of `p`, else the grid cell under `p`.
pub fn snap_station(stations: &[IVec2], p: Vec2) -> IVec2 {
    stations
        .iter()
        .filter(|s| s.as_vec2().distance(p) <= PICK_RADIUS)
        .min_by(|a, b| a.as_vec2().distance(p).total_cmp(&b.as_vec2().distance(p)))
        .copied()
        .unwrap_or_else(|| snap(p))
}

/// Line-drag state: `from` is Some while the button is held after pressing on the map.
#[derive(Resource, Debug, Clone, Default)]
pub struct LineDrag {
    pub from: Option<IVec2>,
    pub to: IVec2,
}

/// Stations placed so far (cells); drag endpoints snap to these.
#[derive(Resource, Debug, Clone, Default)]
pub struct Stations(pub Vec<IVec2>);

/// Button released: register both endpoints as stations and emit the line command.
pub fn finish_drag(stations: &mut Stations, agency: AgencyId, from: IVec2, to: IVec2) -> Option<ToolCommand> {
    if from == to {
        return None;
    }
    for c in [from, to] {
        if !stations.0.contains(&c) {
            stations.0.push(c);
        }
    }
    Some(ToolCommand { agency, action: ToolAction::DrawLine { cells: vec![from, to] } })
}

/// Transit-map route a->b: at most one bend, every leg horizontal, vertical or 45 degrees.
pub fn octilinear(a: Vec2, b: Vec2) -> Vec<Vec2> {
    let d = b - a;
    let diag = d.x.abs().min(d.y.abs());
    if diag == 0.0 || d.x.abs() == d.y.abs() {
        return vec![a, b];
    }
    let bend = a + Vec2::new(d.x.signum(), d.y.signum()) * diag;
    vec![a, bend, b]
}

/// Octilinear route through consecutive station cells (shared joints not repeated).
pub fn route(cells: &[IVec2]) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = vec![];
    for w in cells.windows(2) {
        let leg = octilinear(w[0].as_vec2(), w[1].as_vec2());
        out.extend(leg.into_iter().skip(usize::from(!out.is_empty())));
    }
    out
}

pub fn path_len(p: &[Vec2]) -> f32 {
    p.windows(2).map(|w| w[0].distance(w[1])).sum()
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
    fn ray_hits_flat_and_ramp() {
        let o = Vec3::new(10.0, 50.0, 10.0);
        let d = Vec3::new(1.0, -1.0, 0.0);
        let flat = ray_terrain(o, d, &|_, _| 0.0).unwrap();
        assert!((flat - Vec2::new(60.0, 10.0)).length() < 0.05, "{flat}");
        let ramp = ray_terrain(o, d, &|x, _| 0.5 * x).unwrap();
        assert!((ramp.x - 40.0).abs() < 0.05, "{ramp}");
        assert!(ray_terrain(o, Vec3::Y, &|_, _| 0.0).is_none());
        // straight down (the orthographic view)
        let down = ray_terrain(Vec3::new(3.0, 500.0, -7.0), Vec3::NEG_Y, &|_, _| 0.0).unwrap();
        assert!((down - Vec2::new(3.0, -7.0)).length() < 0.05);
    }

    #[test]
    fn snapping_prefers_existing_station() {
        let st = [IVec2::new(40, 40)];
        assert_eq!(snap_station(&st, Vec2::new(44.0, 38.0)), IVec2::new(40, 40));
        assert_eq!(snap_station(&st, Vec2::new(61.0, 38.0)), IVec2::new(60, 40));
    }

    #[test]
    fn drag_registers_stations_and_emits_line() {
        let mut st = Stations::default();
        assert!(finish_drag(&mut st, AgencyId(1), IVec2::ZERO, IVec2::ZERO).is_none());
        let c = finish_drag(&mut st, AgencyId(1), IVec2::ZERO, IVec2::new(8, 4)).unwrap();
        assert_eq!(c.action, ToolAction::DrawLine { cells: vec![IVec2::ZERO, IVec2::new(8, 4)] });
        assert_eq!(st.0.len(), 2);
    }

    #[test]
    fn routes_are_octilinear() {
        for (a, b) in [(Vec2::ZERO, Vec2::new(30.0, 10.0)), (Vec2::new(5.0, 5.0), Vec2::new(-20.0, -50.0)), (Vec2::ZERO, Vec2::new(0.0, 9.0))] {
            let r = octilinear(a, b);
            assert_eq!((r[0], *r.last().unwrap()), (a, b));
            for w in r.windows(2) {
                let d = w[1] - w[0];
                assert!(d.x == 0.0 || d.y == 0.0 || (d.x.abs() - d.y.abs()).abs() < 1e-4, "{d}");
            }
        }
        let r = route(&[IVec2::ZERO, IVec2::new(30, 10), IVec2::new(30, 40)]);
        assert_eq!(r.len(), 4);
        assert!(path_len(&r) > 60.0);
    }

    #[test]
    fn segment_transform_axes() {
        let t = segment_transform(Vec2::new(0.0, 0.0), Vec2::new(0.0, 10.0), 1.0, 2.0, 0.5);
        assert!((t.scale.x - 10.0).abs() < 1e-5 && t.translation == Vec3::new(0.0, 1.0, 5.0));
        assert!((t.rotation * Vec3::X - Vec3::Z).length() < 1e-5);
    }
}
