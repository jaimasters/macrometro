//! Save / load (R27): the whole game survives a relaunch. The world regenerates from the seed; the save stores what
//! the player and the sim changed: zone growth, built/queued roads, stations, lines, platforms, tracks and money.
//! Autosaves every 30 s and on exit; a missing, unreadable, wrong-version or other-seed save means a fresh game.
//! Reset (button + confirm) deletes the save and relaunches.
use crate::shell::{hop_key, to_transit_with, HopPlan, Hops, Lines, RailGeoms, Razed, RoadClasses, StationMap, PLAYER};
use crate::zones::{GrowthLink, ZoneLayout};
use bevy::ecs::system::SystemParam;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use economy::{Category, Economy};
use serde::{Deserialize, Serialize};
use sim_core::AgencyId;
use transit::{Crossing, CrossingKind, TransitCommands, TransitNet};
use ui_tools::{Stations, ToolAction, ToolCommand};

/// Bump when the format changes; older saves are then ignored (fresh game), never migrated half-way.
pub const SAVE_VERSION: u32 = 1;
const MAGIC: &str = "MMGAME";
const AUTOSAVE_SECS: f32 = 30.0;

/// Growth sim state the city needs to carry on: zone pop/jobs, built roads, roads queued to build.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct GrowthSave {
    pub tick: u64,
    pub zones: Vec<(f64, f64)>,
    pub built: Vec<bool>,
    pub pending: Vec<(usize, u64)>,
    /// Zones razed by eminent domain.
    #[serde(default)]
    pub gone: Vec<usize>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AgencySave {
    pub id: u32,
    pub standing: f64,
    /// (category index, amount spent this period)
    pub spend: Vec<(u8, f64)>,
}

/// One laid rail segment: how the placer routed it (length, speed cap, crossings, arc polyline), measured from `from`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RailSave {
    pub from: (i32, i32),
    pub to: (i32, i32),
    pub len: f64,
    /// None = no speed cap.
    pub vcap: Option<f64>,
    /// (kind index, distance from `from`, road edge)
    pub crossings: Vec<(u8, f64, u32)>,
    pub pts: Vec<(f32, f32)>,
    /// Curve-radius level (index into the placer's radii).
    #[serde(default)]
    pub level: u8,
}

const KINDS: [CrossingKind; 3] = [CrossingKind::Level, CrossingKind::Bridge, CrossingKind::Underpass];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GameSave {
    pub magic: String,
    pub version: u32,
    pub seed: u64,
    pub growth: GrowthSave,
    pub stations: Vec<(i32, i32)>,
    pub lines: Vec<Vec<(i32, i32)>>,
    pub platforms: Vec<((i32, i32), u8)>,
    /// Track count per segment between two stations.
    pub tracks: Vec<((i32, i32), (i32, i32), u8)>,
    /// Placer results per segment (older saves have none: segments are laid straight).
    #[serde(default)]
    pub rails: Vec<RailSave>,
    /// Upgraded road lane classes (edge, class index).
    #[serde(default)]
    pub lanes: Vec<(u32, u8)>,
    pub agencies: Vec<AgencySave>,
}

const CATS: [Category; 4] = [Category::Construction, Category::Acquisition, Category::Operations, Category::RoadAdjacent];

fn cat_index(c: Category) -> u8 {
    CATS.iter().position(|x| *x == c).unwrap_or(0) as u8
}

/// Parse a save; anything wrong with it (garbage, other magic/version/seed) gives None = start fresh.
pub fn parse(text: &str, seed: u64) -> Option<GameSave> {
    let s: GameSave = ron::from_str(text).ok()?;
    (s.magic == MAGIC && s.version == SAVE_VERSION && s.seed == seed).then_some(s)
}

pub fn encode(s: &GameSave) -> String {
    ron::to_string(s).unwrap_or_default()
}

// ---- storage: a file on native, localStorage on wasm ----

#[cfg(not(target_arch = "wasm32"))]
fn save_path() -> std::path::PathBuf {
    // Tests must never read or overwrite the player's real save.
    if cfg!(test) && std::env::var("MM_SAVE").is_err() {
        return std::env::temp_dir().join(format!("macrometro-test-{}.ron", std::process::id()));
    }
    if let Ok(p) = std::env::var("MM_SAVE") {
        return p.into();
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let base = if cfg!(target_os = "macos") { format!("{home}/Library/Application Support") } else { format!("{home}/.local/share") };
    std::path::Path::new(&base).join("macrometro").join("save.ron")
}

#[cfg(not(target_arch = "wasm32"))]
fn read_raw() -> Option<String> {
    std::fs::read_to_string(save_path()).ok()
}

/// Write via a temp file so a kill mid-write never leaves a torn save.
#[cfg(not(target_arch = "wasm32"))]
fn write_raw(text: &str) {
    let p = save_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn clear_raw() {
    let _ = std::fs::remove_file(save_path());
}

#[cfg(target_arch = "wasm32")]
fn store() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}
#[cfg(target_arch = "wasm32")]
fn read_raw() -> Option<String> {
    store()?.get_item("macrometro.save").ok().flatten()
}
#[cfg(target_arch = "wasm32")]
fn write_raw(text: &str) {
    if let Some(s) = store() {
        let _ = s.set_item("macrometro.save", text);
    }
}
#[cfg(target_arch = "wasm32")]
fn clear_raw() {
    if let Some(s) = store() {
        let _ = s.remove_item("macrometro.save");
    }
}

/// The save found at launch (None = fresh game). Inserted before the sim is built so it can resume.
#[derive(Resource, Default)]
pub struct LoadedGame(pub Option<GameSave>);

/// Reads the save at build time (so ZonesPlugin can restore the growth sim); add right after WorldGenPlugin.
pub struct LoadPlugin;

impl Plugin for LoadPlugin {
    fn build(&self, app: &mut App) {
        if app.world().contains_resource::<LoadedGame>() {
            return; // tests (and tools) can hand the game a save directly
        }
        let seed = app.world().get_resource::<world_gen::WorldSeed>().map_or(0, |s| s.0);
        let loaded = read_raw().and_then(|t| parse(&t, seed));
        if loaded.is_none() && read_raw().is_some() {
            warn!("save unreadable or from another version/seed: starting a fresh game");
        }
        app.insert_resource(LoadedGame(loaded));
    }
}

/// Autosave, exit save, restore of stations/lines/money, and the Reset button.
pub struct PersistPlugin;

impl Plugin for PersistPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ResetState>()
            .add_systems(Startup, (restore_game, spawn_reset_ui))
            .add_systems(Update, (autosave, reset_ui))
            .add_systems(Last, save_on_exit);
    }
}

#[derive(SystemParam)]
struct Collect<'w> {
    seed: Res<'w, world_gen::WorldSeed>,
    link: Res<'w, GrowthLink>,
    stations: Res<'w, Stations>,
    lines: Res<'w, Lines>,
    map: Res<'w, StationMap>,
    net: Res<'w, TransitNet>,
    eco: Res<'w, Economy>,
    geoms: Res<'w, RailGeoms>,
    classes: Res<'w, RoadClasses>,
}

impl Collect<'_> {
    fn save(&self) -> Option<GameSave> {
        let growth = self.link.shared.save.lock().ok()?.clone()?;
        let xy = |c: &IVec2| (c.x, c.y);
        let mut tracks: Vec<((i32, i32), (i32, i32), u8)> = vec![];
        for l in &self.lines.all {
            for w in l.cells.windows(2) {
                let edge = self.map.node(w[0]).zip(self.map.node(w[1])).and_then(|(a, b)| self.net.0.rail_edge(a, b));
                if let Some(i) = edge {
                    let (a, b) = (xy(&w[0]).min(xy(&w[1])), xy(&w[0]).max(xy(&w[1])));
                    if !tracks.iter().any(|t| (t.0, t.1) == (a, b)) {
                        tracks.push((a, b, self.net.0.edges[i].tracks));
                    }
                }
            }
        }
        let mut rails: Vec<RailSave> = vec![];
        for l in &self.lines.all {
            for w in l.cells.windows(2) {
                let Some((na, nb)) = self.map.node(w[0]).zip(self.map.node(w[1])) else { continue };
                let Some(i) = self.net.0.rail_edge(na, nb) else { continue };
                let (e, key) = (&self.net.0.edges[i], hop_key(w[0], w[1]));
                if rails.iter().any(|r| hop_key(IVec2::new(r.from.0, r.from.1), IVec2::new(r.to.0, r.to.1)) == key) {
                    continue;
                }
                // the edge runs from its node `a`; find that station's cell to anchor the crossings
                let (from, to) = if e.a == na { (w[0], w[1]) } else { (w[1], w[0]) };
                let mut pts: Vec<Vec2> = self.geoms.0.get(&key).cloned().unwrap_or_default();
                if from != key.0 {
                    pts.reverse();
                }
                rails.push(RailSave {
                    from: xy(&from),
                    to: xy(&to),
                    len: e.len,
                    vcap: e.vcap.is_finite().then_some(e.vcap),
                    crossings: e.crossings.iter().map(|c| (KINDS.iter().position(|k| *k == c.kind).unwrap_or(0) as u8, c.at, c.road)).collect(),
                    pts: pts.iter().map(|p| (p.x, p.y)).collect(),
                    level: self.geoms.1.get(&key).copied().unwrap_or(0),
                });
            }
        }
        Some(GameSave {
            magic: MAGIC.into(),
            version: SAVE_VERSION,
            seed: self.seed.0,
            growth,
            stations: self.stations.0.iter().map(xy).collect(),
            lines: self.lines.all.iter().map(|l| l.cells.iter().map(xy).collect()).collect(),
            platforms: self.stations.0.iter().filter_map(|c| Some((xy(c), self.net.0.platform_count(self.map.node(*c)?)))).collect(),
            tracks,
            rails,
            lanes: { let mut v: Vec<(u32, u8)> = self.classes.0.iter().map(|(e, c)| (*e as u32, *c as u8)).collect(); v.sort(); v },
            agencies: self
                .eco
                .0
                .agencies
                .iter()
                .map(|(id, a)| AgencySave { id: id.0, standing: a.standing, spend: a.spend.iter().map(|(c, v)| (cat_index(*c), *v)).collect() })
                .collect(),
        })
    }
}

/// Replay a loaded game into the live resources (no charges: money is restored from the save).
#[allow(clippy::too_many_arguments)]
fn restore_game(
    loaded: Res<LoadedGame>,
    layout: Res<ZoneLayout>,
    mut stations: ResMut<Stations>,
    mut map: ResMut<StationMap>,
    mut lines: ResMut<Lines>,
    mut tc: ResMut<TransitCommands>,
    mut eco: ResMut<Economy>,
    mut geoms: ResMut<RailGeoms>,
    mut razed: ResMut<Razed>,
    mut classes: ResMut<RoadClasses>,
    link: Res<crate::zones::GrowthLink>,
) {
    let Some(g) = &loaded.0 else { return };
    for &(e, c) in &g.lanes {
        classes.0.insert(e as usize, city::lanes::LANE_CLASSES[(c as usize).min(6)]);
        link.shared.lanes.lock().unwrap().push((e as usize, c));
    }
    razed.0 = vec![false; layout.0.len()];
    for &z in &g.growth.gone {
        if let Some(r) = razed.0.get_mut(z) {
            *r = true;
        }
    }
    let centres: Vec<Vec2> = layout.0.iter().map(|z| z.center).collect();
    let cell = |c: &(i32, i32)| IVec2::new(c.0, c.1);
    // segments exactly as the placer laid them
    let mut hops = Hops::new();
    for r in &g.rails {
        let (from, to) = (cell(&r.from), cell(&r.to));
        let crossings = r.crossings.iter().map(|(k, at, road)| Crossing { kind: KINDS[(*k as usize).min(2)], at: *at, road: *road }).collect();
        hops.insert(hop_key(from, to), Some(HopPlan { from, len: r.len, vcap: r.vcap.unwrap_or(f64::INFINITY), crossings }));
        if !r.pts.is_empty() {
            let mut pts: Vec<Vec2> = r.pts.iter().map(|p| Vec2::new(p.0, p.1)).collect();
            if from != hop_key(from, to).0 {
                pts.reverse();
            }
            geoms.0.insert(hop_key(from, to), pts);
        }
        geoms.1.insert(hop_key(from, to), r.level);
    }
    let run = |action: ToolAction, tc: &mut TransitCommands, map: &mut StationMap| {
        tc.0.extend(to_transit_with(&ToolCommand { agency: PLAYER, action }, map, &centres, &hops).0);
    };
    for c in g.stations.iter().map(cell) {
        stations.0.push(c);
        run(ToolAction::PlaceStation { cell: c }, &mut tc, &mut map);
    }
    for l in &g.lines {
        let cells: Vec<IVec2> = l.iter().map(cell).collect();
        if cells.len() >= 2 && cells.iter().all(|c| stations.0.contains(c)) {
            run(ToolAction::DrawLine { cells: cells.clone() }, &mut tc, &mut map);
            lines.add(cells);
        }
    }
    for (c, n) in &g.platforms {
        for _ in 1..*n {
            run(ToolAction::AddPlatform { cell: cell(c) }, &mut tc, &mut map);
        }
    }
    for (a, b, t) in &g.tracks {
        if *t >= 4 {
            run(ToolAction::UpgradeTracks { a: cell(a), b: cell(b) }, &mut tc, &mut map);
        }
    }
    for a in &g.agencies {
        if let Some(st) = eco.0.agencies.get_mut(&AgencyId(a.id)) {
            st.standing = a.standing;
            st.spend = a.spend.iter().filter_map(|(c, v)| CATS.get(*c as usize).map(|c| (*c, *v))).collect();
        }
    }
}

#[derive(Resource, Default)]
struct ResetState {
    confirming: bool,
    /// Set once Reset is confirmed: nothing may write the old game back.
    done: bool,
}

fn autosave(time: Res<Time<Real>>, mut since: Local<f32>, reset: Res<ResetState>, c: Collect) {
    *since += time.delta_secs();
    if *since < AUTOSAVE_SECS || reset.done {
        return;
    }
    *since = 0.0;
    if let Some(s) = c.save() {
        write_raw(&encode(&s));
    }
}

fn save_on_exit(mut exit: MessageReader<AppExit>, reset: Res<ResetState>, c: Collect) {
    if exit.read().next().is_some() && !reset.done {
        if let Some(s) = c.save() {
            write_raw(&encode(&s));
        }
    }
}

// ---- Reset button + confirm ----

#[derive(Component)]
struct ResetButton;
#[derive(Component)]
struct ResetDialog;
#[derive(Component, PartialEq, Eq, Clone, Copy)]
enum DialogButton {
    Yes,
    No,
}

const INK: Color = Color::srgb(0.12, 0.13, 0.16);
const PANEL: Color = Color::srgba(1.0, 1.0, 1.0, 0.96);

fn spawn_reset_ui(mut commands: Commands) {
    let btn = |text: &str, colour: Color| {
        (
            Hovered::default(),
            Node { padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)), border_radius: BorderRadius::all(Val::Px(10.0)), ..default() },
            BackgroundColor(colour),
            children![(Text::new(text.to_string()), TextFont { font: crate::hud::ui_font(), font_size: FontSize::Px(14.0), ..default() }, TextColor(INK))],
        )
    };
    commands.spawn((
        ResetButton,
        Node { position_type: PositionType::Absolute, top: Val::Px(52.0), right: Val::Px(10.0), ..default() },
        children![btn("Reset city", Color::srgba(1.0, 1.0, 1.0, 0.85))],
    ));
    commands
        .spawn((
            ResetDialog,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
        ))
        .with_children(|d| {
            d.spawn((
                Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(12.0), padding: UiRect::all(Val::Px(18.0)), border_radius: BorderRadius::all(Val::Px(14.0)), ..default() },
                BackgroundColor(PANEL),
            ))
            .with_children(|p| {
                p.spawn((Text::new("Start a fresh city? Your current city is deleted."), TextFont { font: crate::hud::ui_font(), font_size: FontSize::Px(16.0), ..default() }, TextColor(INK)));
                p.spawn(Node { column_gap: Val::Px(10.0), justify_content: JustifyContent::Center, ..default() }).with_children(|r| {
                    r.spawn((DialogButton::Yes, btn("Reset", Color::srgb(0.95, 0.65, 0.62))));
                    r.spawn((DialogButton::No, btn("Cancel", Color::srgb(0.90, 0.92, 0.96))));
                });
            });
        });
}

fn reset_ui(
    mouse: Res<ButtonInput<MouseButton>>,
    mut state: ResMut<ResetState>,
    open: Query<&Children, With<ResetButton>>,
    hover: Query<&Hovered>,
    dialog_btns: Query<(&DialogButton, &Hovered)>,
    mut dialog: Query<&mut Node, With<ResetDialog>>,
    mut exit: MessageWriter<AppExit>,
) {
    let want = if state.confirming { Display::Flex } else { Display::None };
    for mut n in &mut dialog {
        if n.display != want {
            n.display = want;
        }
    }
    if !mouse.just_released(MouseButton::Left) || state.done {
        return;
    }
    let button_hot = open.iter().any(|ch| ch.iter().any(|e| hover.get(e).is_ok_and(|h| h.get())));
    if !state.confirming {
        state.confirming = button_hot;
        return;
    }
    match dialog_btns.iter().find(|(_, h)| h.get()).map(|(b, _)| *b) {
        Some(DialogButton::Yes) => {
            state.done = true;
            clear_raw();
            relaunch();
            exit.write(AppExit::Success);
        }
        Some(DialogButton::No) | None => state.confirming = false,
    }
}

/// A fresh process = a fresh city (sim thread, ids and visuals all restart cleanly).
#[cfg(not(target_arch = "wasm32"))]
fn relaunch() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(std::env::args().skip(1)).spawn();
    }
}

#[cfg(target_arch = "wasm32")]
fn relaunch() {
    if let Some(w) = web_sys::window() {
        let _ = w.location().reload();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> GameSave {
        GameSave {
            magic: MAGIC.into(),
            version: SAVE_VERSION,
            seed: 7,
            growth: GrowthSave { tick: 1234, zones: vec![(12.5, 3.25), (0.0, 0.0)], built: vec![true, false, true], pending: vec![(1, 1300)], gone: vec![1] },
            stations: vec![(0, 12), (40, 12)],
            lines: vec![vec![(0, 12), (40, 12)]],
            platforms: vec![((0, 12), 3), ((40, 12), 1)],
            tracks: vec![((0, 12), (40, 12), 4)],
            lanes: vec![(2, 3)],
            rails: vec![RailSave { from: (0, 12), to: (40, 12), len: 41.5, vcap: Some(6.3), crossings: vec![(0, 20.0, 3)], pts: vec![(0.0, 12.0), (40.0, 12.0)], level: 1 }],
            agencies: vec![AgencySave { id: 1, standing: 0.42, spend: vec![(0, 55.5), (2, 1.0)] }],
        }
    }

    #[test]
    fn save_round_trips_exactly() {
        let s = sample();
        assert_eq!(parse(&encode(&s), 7), Some(s));
    }

    #[test]
    fn bad_saves_start_a_fresh_game_instead_of_crashing() {
        let s = sample();
        assert_eq!(parse(&encode(&s), 8), None, "another world");
        let mut old = s.clone();
        old.version = SAVE_VERSION + 1;
        assert_eq!(parse(&encode(&old), 7), None, "other version");
        let mut wrong = s.clone();
        wrong.magic = "NOPE".into();
        assert_eq!(parse(&encode(&wrong), 7), None);
        for junk in ["", "garbage", "(", "GameSave(magic: 1)"] {
            assert_eq!(parse(junk, 7), None);
        }
        let truncated = &encode(&s)[..40];
        assert_eq!(parse(truncated, 7), None);
    }

    fn boot(loaded: Option<GameSave>) -> App {
        use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
        let mut app = App::new();
        app.insert_resource(LoadedGame(loaded))
            .add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            .add_plugins(bevy::audio::AudioPlugin::default())
            .add_plugins(crate::MacrometroPlugins);
        app.update();
        app.world_mut().run_schedule(FixedUpdate);
        app
    }

    fn collect(app: &mut App) -> GameSave {
        use bevy::ecs::system::RunSystemOnce;
        app.world_mut().run_system_once(|c: Collect| c.save()).unwrap().expect("growth state available")
    }

    /// Build something, save, relaunch from the save, and get the same city back.
    #[test]
    fn a_saved_game_comes_back_after_relaunch() {
        let mut a = boot(None);
        let centre = a.world().resource::<ZoneLayout>().0[0].center.as_ivec2();
        let cells = vec![centre, centre + IVec2::new(30, 0), centre + IVec2::new(60, 10)];
        a.world_mut().resource_mut::<Stations>().0.extend(cells.clone());
        for action in [
            ToolAction::DrawLine { cells: cells.clone() },
            ToolAction::AddPlatform { cell: cells[0] },
            ToolAction::UpgradeTracks { a: cells[0], b: cells[1] },
        ] {
            a.world_mut().write_message(ToolCommand { agency: PLAYER, action });
            a.update();
            a.world_mut().run_schedule(FixedUpdate);
        }
        // the sim thread refreshes its copy every 5 s; seed one for the test
        let snap = a.world().resource::<GrowthLink>().snapshots.latest();
        let n = snap.pop.len();
        let growth = GrowthSave { tick: 321, zones: (0..n).map(|i| (i as f64, 0.5)).collect(), built: snap.built.clone(), pending: vec![], gone: vec![] };
        *a.world().resource::<GrowthLink>().shared.save.lock().unwrap() = Some(growth);
        let saved = collect(&mut a);
        assert_eq!(saved.stations, cells.iter().map(|c| (c.x, c.y)).collect::<Vec<_>>());
        assert_eq!(saved.platforms[0], ((cells[0].x, cells[0].y), 2));
        assert!(saved.tracks.iter().any(|t| t.2 == 4), "{:?}", saved.tracks);
        assert!(saved.agencies.iter().any(|g| g.id == PLAYER.0 && !g.spend.is_empty()));

        let mut b = boot(Some(parse(&encode(&saved), saved.seed).expect("valid save")));
        b.world_mut().run_schedule(FixedUpdate);
        assert_eq!(b.world().resource::<Stations>().0, cells);
        assert_eq!(b.world().resource::<Lines>().all.len(), 1);
        let net = &b.world().resource::<TransitNet>().0;
        assert_eq!(net.lines.len(), 1);
        assert!(b.world().resource::<transit::TransitErrors>().0.is_empty());
        assert_eq!(collect(&mut b), saved, "save -> load -> save is stable");
        // a bad save must not stop a fresh game booting
        assert!(boot(None).world().resource::<Stations>().0.is_empty());
    }

    #[test]
    fn category_indices_are_stable() {
        for (i, c) in CATS.iter().enumerate() {
            assert_eq!(cat_index(*c) as usize, i);
        }
    }
}
