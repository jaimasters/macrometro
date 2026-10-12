//! R30: click a road or rail segment (Pan tool) for a menu like the station menu. Roads: lane-class upgrade (cost,
//! razing). Rails: +Track, wider curve, bridge crossings, and an estimated load. Esc or clicking elsewhere closes it.
use crate::hud::ui_font;
use crate::shell::{crossing_upgrades, curve_upgrade, line_geometry, road_upgrade, Lines, RailGeoms, Razed, RoadClasses, StationMap};
use crate::stations::SelectedStation;
use crate::zones::{CityPlan, GrowthLink, ZoneLayout};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use city::placer::Ctx;
use transit::{Edge, TransitNet};
use ui_tools::{station_at, ActiveAgency, ActiveTool, CursorWorld, Stations, ToolAction, ToolCommand};

const PICK: f32 = 2.5;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Segment {
    Road(usize),
    Rail(IVec2, IVec2),
}

#[derive(Resource, Default)]
pub struct SelectedSegment(pub Option<Segment>);

#[derive(Component)]
struct SegMenu;
#[derive(Component)]
struct SegButton(ToolAction);

pub fn dist_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() < 1e-9 { 0.0 } else { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) };
    p.distance(a + ab * t)
}

/// Nearest rail hop (station pair) to `p` within `PICK`, with its distance.
pub fn rail_at(lines: &Lines, geoms: &RailGeoms, p: Vec2) -> Option<(IVec2, IVec2, f32)> {
    let mut best: Option<(IVec2, IVec2, f32)> = None;
    for l in &lines.all {
        let (path, idx) = line_geometry(&l.cells, geoms);
        for (h, w) in l.cells.windows(2).enumerate() {
            let d = path[idx[h]..=idx[h + 1]].windows(2).map(|s| dist_to_segment(p, s[0], s[1])).fold(f32::INFINITY, f32::min);
            if d <= PICK && best.is_none_or(|b| d < b.2) {
                best = Some((w[0], w[1], d));
            }
        }
    }
    best
}

/// Nearest built road edge to `p` within `PICK`.
pub fn road_at(plan: &city::TownPlan, built: &[bool], p: Vec2) -> Option<(usize, f32)> {
    plan.graph
        .edges
        .iter()
        .enumerate()
        .filter(|(i, _)| built.get(*i).copied().unwrap_or(false))
        .map(|(i, e)| (i, dist_to_segment(p, plan.graph.nodes[e.a as usize], plan.graph.nodes[e.b as usize])))
        .filter(|(_, d)| *d <= PICK)
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

pub struct SegmentMenuPlugin;

impl Plugin for SegmentMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SelectedSegment>().add_systems(Update, (pick_segment, rebuild_menu, menu_clicks).chain());
    }
}

#[allow(clippy::too_many_arguments)]
fn pick_segment(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    tool: Res<ActiveTool>,
    cursor: Res<CursorWorld>,
    stations: Res<Stations>,
    lines: Res<Lines>,
    geoms: Res<RailGeoms>,
    cp: Res<CityPlan>,
    link: Res<GrowthLink>,
    hovered: Query<&Hovered, With<SegButton>>,
    mut sel: ResMut<SelectedSegment>,
    mut station: ResMut<SelectedStation>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        sel.0 = None;
    }
    if *tool != ActiveTool::Pan || !mouse.just_pressed(MouseButton::Left) || hovered.iter().any(|h| h.get()) {
        return;
    }
    let Some(p) = cursor.0 else { return };
    if station_at(&stations.0, p).is_some() {
        sel.0 = None;
        return;
    }
    let rail = rail_at(&lines, &geoms, p);
    let road = road_at(&cp.plan, &link.snapshots.latest().built, p);
    sel.0 = match (rail, road) {
        (Some(r), Some(d)) if d.1 < r.2 => Some(Segment::Road(d.0)),
        (Some(r), _) => Some(Segment::Rail(r.0, r.1)),
        (None, Some(d)) => Some(Segment::Road(d.0)),
        (None, None) => None,
    };
    if sel.0.is_some() {
        station.0 = None;
    }
}

fn text(s: impl Into<String>, size: f32, c: Color) -> impl Bundle {
    (Text::new(s), TextFont { font: ui_font(), font_size: FontSize::Px(size), ..default() }, TextColor(c))
}

#[allow(clippy::too_many_arguments)]
fn rebuild_menu(
    sel: Res<SelectedSegment>,
    link: Res<GrowthLink>,
    cp: Res<CityPlan>,
    razed: Res<Razed>,
    classes: Res<RoadClasses>,
    geoms: Res<RailGeoms>,
    lines: Res<Lines>,
    net: Res<TransitNet>,
    map: Res<StationMap>,
    _layout: Res<ZoneLayout>,
    seed: Res<world_gen::WorldSeed>,
    trains: Res<crate::rails::Trains>,
    old: Query<Entity, With<SegMenu>>,
    mut commands: Commands,
    mut last: Local<Option<(Option<Segment>, u64)>>,
) {
    let snap = link.snapshots.latest();
    // refresh on selection change and about once a second of sim (tick) so cost and load stay current
    let key = (sel.0, snap.tick / 10);
    if *last == Some(key) && !net.is_changed() && !classes.is_changed() && !geoms.is_changed() {
        return;
    }
    *last = Some(key);
    for e in &old {
        commands.entity(e).despawn();
    }
    let Some(seg) = sel.0 else { return };
    let gone: Vec<bool> = (0..cp.plan.parcels.len()).map(|i| razed.0.get(i).copied().unwrap_or(false)).collect();
    let land = |p: Vec2| city::is_land(seed.0, p);
    let ctx = Ctx { plan: &cp.plan, built: &snap.built, pop: &snap.pop, gone: &gone, land: &land };
    let mut rows: Vec<(String, Option<ToolAction>)> = vec![];
    let title;
    match seg {
        Segment::Road(e) => {
            let ed = &cp.plan.graph.edges[e];
            let cls = classes.0.get(&e).copied().unwrap_or_else(|| city::lanes::default_class(ed.class));
            title = format!("Road: {} ({} lanes)", cls.name(), cls.lanes());
            rows.push((format!("Traffic {:.0}% of capacity", snap.vc.get(e).copied().unwrap_or(0.0) * 100.0), None));
            match road_upgrade(&ctx, &classes, e) {
                Some(u) => {
                    let razing = if u.demolition.zones.is_empty() { String::new() } else { format!(", razes {}", u.demolition.zones.len()) };
                    rows.push((format!("Widen to {}  ({:.0}{razing})", u.to.name(), u.cost), Some(ToolAction::UpgradeRoad { edge: e })));
                }
                None => rows.push(("Widest class".into(), None)),
            }
        }
        Segment::Rail(a, b) => {
            title = "Track".into();
            let (na, nb) = (map.node(a), map.node(b));
            if let Some(i) = na.zip(nb).and_then(|(x, y)| net.0.rail_edge(x, y)) {
                let e = &net.0.edges[i];
                let serving: Vec<u32> = lines.all.iter().filter(|l| l.cells.windows(2).any(|w| (w[0], w[1]) == (a, b) || (w[0], w[1]) == (b, a))).map(|l| l.id as u32).collect();
                let on: Vec<f64> = trains.0.trains.iter().filter(|t| serving.contains(&t.line.0)).map(|t| t.load).collect();
                let mean = if on.is_empty() { 0.0 } else { on.iter().sum::<f64>() / on.len() as f64 };
                rows.push((format!("{} tracks, {} train(s), load {:.0}%", e.tracks, on.len(), mean / transit::trains::TRAIN_CAPACITY * 100.0), None));
                for l in &serving {
                    let have = trains.0.extra.get(l).copied().unwrap_or(0);
                    if trains.0.siding_free(transit::LineId(*l)) == 0 {
                        rows.push((format!("Line {}: buy a siding to add trains", l + 1), None));
                    } else if have < transit::trains::MAX_EXTRA_TRAINS {
                        rows.push((format!("+ Train on line {}  ({:.0}, {} bought)", l + 1, crate::shell::TRAIN_COST, have), Some(ToolAction::AddTrain { line: *l as usize })));
                    }
                }
                if let Some(c) = Edge::track_cost_between(&net.0, na.unwrap(), nb.unwrap()) {
                    rows.push((format!("+ Track  ({c:.0})"), Some(ToolAction::UpgradeTracks { a, b })));
                }
            }
            if let Some(u) = curve_upgrade(&ctx, &geoms, &net.0, &map, a, b) {
                let razing = if u.placement.demolition.zones.is_empty() { String::new() } else { format!(", razes {}", u.placement.demolition.zones.len()) };
                rows.push((format!("Wider curve  ({:.0}{razing})", u.cost), Some(ToolAction::UpgradeCurve { a, b })));
            }
            if let Some((_, ups, cost)) = crossing_upgrades(&net.0, &map, a, b, false) {
                rows.push((format!("Bridge {} crossing(s)  ({cost:.0})", ups.len()), Some(ToolAction::UpgradeCrossings { a, b, underpass: false })));
            }
        }
    }
    commands
        .spawn((
            SegMenu,
            Hovered::default(),
            Node {
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(10.0)),
                bottom: Val::Px(60.0),
                right: Val::Px(14.0),
                min_width: Val::Px(220.0),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.95)),
        ))
        .with_children(|m| {
            m.spawn(text(title, 16.0, Color::srgb(0.12, 0.13, 0.16)));
            for (t, act) in rows {
                match act {
                    None => {
                        m.spawn(text(t, 13.0, Color::srgb(0.4, 0.42, 0.47)));
                    }
                    Some(a) => {
                        m.spawn((
                            SegButton(a),
                            Hovered::default(),
                            Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)), border_radius: BorderRadius::all(Val::Px(8.0)), ..default() },
                            BackgroundColor(Color::srgb(0.93, 0.95, 0.98)),
                        ))
                        .with_child(text(t, 14.0, Color::srgb(0.12, 0.13, 0.16)));
                    }
                }
            }
        });
}

fn menu_clicks(mouse: Res<ButtonInput<MouseButton>>, agency: Res<ActiveAgency>, buttons: Query<(&SegButton, &Hovered)>, mut out: MessageWriter<ToolCommand>) {
    if mouse.just_released(MouseButton::Left) {
        if let Some((b, _)) = buttons.iter().find(|(_, h)| h.get()) {
            out.write(ToolCommand { agency: agency.0, action: b.0.clone() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_to_a_segment_clamps_to_its_ends() {
        let (a, b) = (Vec2::ZERO, Vec2::new(10.0, 0.0));
        assert_eq!(dist_to_segment(Vec2::new(5.0, 3.0), a, b), 3.0);
        assert_eq!(dist_to_segment(Vec2::new(-4.0, 3.0), a, b), 5.0);
    }
}
