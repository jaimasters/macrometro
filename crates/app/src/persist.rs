//! Save / load (R27): the whole game survives a relaunch. The world regenerates from the seed; the save stores what
//! the player and the sim changed: zone growth, built/queued roads, stations, lines, platforms, tracks and money.
//! Autosaves every 30 s and on exit; a missing, unreadable, wrong-version or other-seed save means a fresh game.
//! Reset (button + confirm) deletes the save and relaunches.
use crate::shell::{to_transit, Lines, StationMap, PLAYER};
use crate::zones::{GrowthLink, ZoneLayout};
use bevy::ecs::system::SystemParam;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use economy::{Category, Economy};
use serde::{Deserialize, Serialize};
use sim_core::AgencyId;
use transit::{TransitCommands, TransitNet};
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
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AgencySave {
    pub id: u32,
    pub standing: f64,
    /// (category index, amount spent this period)
    pub spend: Vec<(u8, f64)>,
}

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
        Some(GameSave {
            magic: MAGIC.into(),
            version: SAVE_VERSION,
            seed: self.seed.0,
            growth,
            stations: self.stations.0.iter().map(xy).collect(),
            lines: self.lines.all.iter().map(|l| l.cells.iter().map(xy).collect()).collect(),
            platforms: self.stations.0.iter().filter_map(|c| Some((xy(c), self.net.0.platform_count(self.map.node(*c)?)))).collect(),
            tracks,
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
) {
    let Some(g) = &loaded.0 else { return };
    let centres: Vec<Vec2> = layout.0.iter().map(|z| z.center).collect();
    let cell = |c: &(i32, i32)| IVec2::new(c.0, c.1);
    let run = |action: ToolAction, tc: &mut TransitCommands, map: &mut StationMap| {
        tc.0.extend(to_transit(&ToolCommand { agency: PLAYER, action }, map, &centres).0);
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
            growth: GrowthSave { tick: 1234, zones: vec![(12.5, 3.25), (0.0, 0.0)], built: vec![true, false, true], pending: vec![(1, 1300)] },
            stations: vec![(0, 12), (40, 12)],
            lines: vec![vec![(0, 12), (40, 12)]],
            platforms: vec![((0, 12), 3), ((40, 12), 1)],
            tracks: vec![((0, 12), (40, 12), 4)],
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
        let growth = GrowthSave { tick: 321, zones: (0..n).map(|i| (i as f64, 0.5)).collect(), built: snap.built.clone(), pending: vec![] };
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
