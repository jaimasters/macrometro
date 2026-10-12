//! Station-centric interaction (B2): click a station (Pan tool) and a small menu opens beside it:
//! info (name, people waiting, lines, platforms) and actions: extend a line from this terminus, start a new line,
//! add a platform, delete. The palette tools stay as shortcuts. Esc or clicking elsewhere closes it.
use crate::camera::MapCam;
use crate::hud::ui_font;
use crate::layout::station_name;
use crate::shell::{line_colour, Lines, StationMap, PLATFORM_COST};
use crate::stations::SelectedStation;
use crate::zones::{GrowthLink, ZoneLayout};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use transit::{TransitNet, MAX_PLATFORMS};
use ui_tools::{ActiveAgency, ActiveTool, ExtendState, LineExtend, Stations, ToolAction, ToolCommand};

const INK: Color = Color::srgb(0.12, 0.13, 0.16);
const PANEL: Color = Color::srgba(1.0, 1.0, 1.0, 0.95);
const BTN: Color = Color::srgb(0.93, 0.95, 0.98);
const BTN_HOT: Color = Color::srgb(0.80, 0.88, 1.0);
const MUTED: Color = Color::srgb(0.40, 0.42, 0.47);
/// People waiting per resident who would ride (a few minutes of arrivals). Estimate until trains carry real queues.
const WAIT_PER_POP: f32 = 0.02;

#[derive(Component)]
struct MenuRoot(IVec2);

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum MenuAction {
    Extend(usize),
    NewLine,
    Platform,
    Delete,
    Close,
}

#[derive(Component)]
struct MenuButton(MenuAction);

#[derive(Component)]
struct WaitingText;

/// Estimated people waiting at a station at `p`: transit-bound residents of zones within feeder range.
/// `zones` = (centre, population, transit share).
pub fn waiting_estimate(p: Vec2, zones: &[(Vec2, f32, f32)]) -> u32 {
    zones.iter().filter(|z| z.0.distance(p) <= city::MAX_ACCESS).map(|z| z.1 * z.2 * WAIT_PER_POP).sum::<f32>().round() as u32
}

/// Lines serving `cell` (any stop).
pub fn lines_serving(lines: &Lines, cell: IVec2) -> Vec<usize> {
    lines.all.iter().filter(|l| l.cells.contains(&cell)).map(|l| l.id).collect()
}

/// Menu rows for a station: (action, label) in display order.
pub fn menu_actions(lines: &Lines, platforms: u8, cell: IVec2) -> Vec<(MenuActionKind, String)> {
    let mut v: Vec<(MenuActionKind, String)> =
        lines.terminating_at(cell).into_iter().map(|l| (MenuActionKind::Extend(l), format!("Extend line {} from here", l + 1))).collect();
    v.push((MenuActionKind::NewLine, "New line from here".into()));
    if platforms < MAX_PLATFORMS {
        v.push((MenuActionKind::Platform, format!("+ Platform  ({PLATFORM_COST:.0})")));
    }
    v.push((MenuActionKind::Delete, "Delete station".into()));
    v
}

/// Public mirror of the menu actions (tests and callers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuActionKind {
    Extend(usize),
    NewLine,
    Platform,
    Delete,
}

impl From<MenuActionKind> for MenuAction {
    fn from(k: MenuActionKind) -> Self {
        match k {
            MenuActionKind::Extend(l) => MenuAction::Extend(l),
            MenuActionKind::NewLine => MenuAction::NewLine,
            MenuActionKind::Platform => MenuAction::Platform,
            MenuActionKind::Delete => MenuAction::Delete,
        }
    }
}

pub struct StationMenuPlugin;

impl Plugin for StationMenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (rebuild_menu, update_menu, menu_clicks).chain());
    }
}

fn label(s: impl Into<String>, size: f32, colour: Color) -> impl Bundle {
    (Text::new(s), TextFont { font: ui_font(), font_size: FontSize::Px(size), ..default() }, TextColor(colour))
}

#[allow(clippy::too_many_arguments)]
fn rebuild_menu(
    sel: Res<SelectedStation>,
    lines: Res<Lines>,
    net: Res<TransitNet>,
    map: Res<StationMap>,
    layout: Res<ZoneLayout>,
    seed: Res<world_gen::WorldSeed>,
    old: Query<Entity, With<MenuRoot>>,
    mut commands: Commands,
) {
    if !(sel.is_changed() || lines.is_changed() || net.is_changed()) {
        return;
    }
    for e in &old {
        commands.entity(e).despawn();
    }
    let Some(cell) = sel.0 else { return };
    let platforms = map.node(cell).map_or(1, |n| net.0.platform_count(n));
    let name = station_name(seed.0, &layout.0, cell.as_vec2());
    let serving = lines_serving(&lines, cell);
    commands
        .spawn((
            MenuRoot(cell),
            Hovered::default(),
            Node {
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(10.0)),
                min_width: Val::Px(190.0),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(PANEL),
        ))
        .with_children(|m| {
            m.spawn(Node { justify_content: JustifyContent::SpaceBetween, column_gap: Val::Px(12.0), ..default() }).with_children(|r| {
                r.spawn(label(name, 16.0, INK));
                button(r, MenuAction::Close, "x");
            });
            m.spawn((WaitingText, label("Waiting 0", 13.0, MUTED)));
            m.spawn(Node { column_gap: Val::Px(6.0), flex_wrap: FlexWrap::Wrap, ..default() }).with_children(|r| {
                r.spawn(label("Lines", 13.0, MUTED));
                if serving.is_empty() {
                    r.spawn(label("none yet", 13.0, MUTED));
                }
                for l in &serving {
                    r.spawn(label(format!("{}", l + 1), 14.0, line_colour(*l)));
                }
            });
            m.spawn(label(format!("Platforms {platforms}"), 13.0, MUTED));
            for (k, text) in menu_actions(&lines, platforms, cell) {
                button(m, k.into(), &text);
            }
        });
}

fn button(parent: &mut ChildSpawnerCommands, action: MenuAction, text: &str) {
    parent
        .spawn((
            MenuButton(action),
            Hovered::default(),
            Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)), border_radius: BorderRadius::all(Val::Px(8.0)), ..default() },
            BackgroundColor(BTN),
        ))
        .with_child(label(text, 14.0, INK));
}

/// Follow the station on screen; refresh the waiting estimate; hover colours.
fn update_menu(
    cam: Option<Res<MapCam>>,
    camera: Option<Single<(&Camera, &GlobalTransform), With<Camera3d>>>,
    layout: Res<ZoneLayout>,
    link: Res<GrowthLink>,
    mut roots: Query<(&MenuRoot, &mut Node)>,
    mut waiting: Query<&mut Text, With<WaitingText>>,
    mut buttons: Query<(&Hovered, &mut BackgroundColor), With<MenuButton>>,
) {
    for (h, mut bg) in &mut buttons {
        let want = if h.get() { BTN_HOT } else { BTN };
        if bg.0 != want {
            bg.0 = want;
        }
    }
    if roots.is_empty() {
        return;
    }
    let s = link.snapshots.latest();
    let (Some(_), Some(camera)) = (cam, camera) else { return };
    let (camera, tf) = *camera;
    for (r, mut node) in &mut roots {
        let p = r.0.as_vec2();
        if let Ok(v) = camera.world_to_viewport(tf, Vec3::new(p.x, 0.0, p.y)) {
            node.left = Val::Px(v.x + 26.0);
            node.top = Val::Px((v.y - 30.0).max(8.0));
        }
        let zones: Vec<(Vec2, f32, f32)> =
            layout.0.iter().enumerate().map(|(i, g)| (g.center, s.pop.get(i).copied().unwrap_or(0.0), s.zone_share.get(i).copied().unwrap_or(0.0))).collect();
        let line = format!("Waiting ~{}", waiting_estimate(p, &zones));
        for mut t in &mut waiting {
            if t.0 != line {
                t.0 = line.clone();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn menu_clicks(
    mouse: Res<ButtonInput<MouseButton>>,
    agency: Res<ActiveAgency>,
    mut sel: ResMut<SelectedStation>,
    mut tool: ResMut<ActiveTool>,
    mut extend: ResMut<LineExtend>,
    mut stations: ResMut<Stations>,
    mut out: MessageWriter<ToolCommand>,
    buttons: Query<(&MenuButton, &Hovered)>,
) {
    if !mouse.just_released(MouseButton::Left) {
        return;
    }
    let (Some(cell), Some((b, _))) = (sel.0, buttons.iter().find(|(_, h)| h.get())) else { return };
    match b.0 {
        MenuAction::Extend(line) => {
            extend.0 = Some(ExtendState { line: Some(line), from: cell, to: cell });
            *tool = ActiveTool::DrawLine;
            sel.0 = None;
        }
        MenuAction::NewLine => {
            extend.0 = Some(ExtendState { line: None, from: cell, to: cell });
            *tool = ActiveTool::DrawLine;
            sel.0 = None;
        }
        MenuAction::Platform => {
            out.write(ToolCommand { agency: agency.0, action: ToolAction::AddPlatform { cell } });
        }
        MenuAction::Delete => {
            stations.0.retain(|s| *s != cell);
            out.write(ToolCommand { agency: agency.0, action: ToolAction::DeleteStation { cell } });
            sel.0 = None;
        }
        MenuAction::Close => sel.0 = None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::Lines;

    #[test]
    fn extend_is_offered_only_at_a_terminus_and_platform_hides_at_the_cap() {
        let (a, b, c) = (IVec2::new(0, 0), IVec2::new(40, 0), IVec2::new(80, 0));
        let mut lines = Lines::default();
        lines.add(vec![a, b, c]);
        let labels = |cell, plats| menu_actions(&lines, plats, cell).into_iter().map(|(k, _)| k).collect::<Vec<_>>();
        assert_eq!(labels(a, 1), vec![MenuActionKind::Extend(0), MenuActionKind::NewLine, MenuActionKind::Platform, MenuActionKind::Delete]);
        assert_eq!(labels(b, 1), vec![MenuActionKind::NewLine, MenuActionKind::Platform, MenuActionKind::Delete], "mid-line stop: no extend");
        assert!(!labels(c, MAX_PLATFORMS).contains(&MenuActionKind::Platform));
        assert_eq!(lines_serving(&lines, b), vec![0]);
        assert!(lines_serving(&lines, IVec2::new(1, 1)).is_empty());
    }

    #[test]
    fn waiting_follows_nearby_riders_only() {
        let zones = [(Vec2::new(5.0, 0.0), 1000.0, 0.5), (Vec2::new(500.0, 0.0), 1000.0, 1.0)];
        assert_eq!(waiting_estimate(Vec2::ZERO, &zones), 10);
        assert_eq!(waiting_estimate(Vec2::ZERO, &[(Vec2::new(5.0, 0.0), 1000.0, 0.0)]), 0, "no transit, nobody waits");
    }
}
