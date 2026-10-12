//! bevy_ui HUD: budget/standing chips (top right), tool palette with icons + hover hints (bottom centre),
//! one-line contextual hint above it. Wraps rather than clips at small window sizes.
use crate::shell::budget_summary;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use economy::{CurrentBudget, Economy};
use sound::SoundSettings;
use ui_tools::{ActiveTool, LineExtend};

const INK: Color = Color::srgb(0.12, 0.13, 0.16);
const PANEL: Color = Color::srgba(1.0, 1.0, 1.0, 0.92);
const SELECTED: Color = Color::srgb(0.80, 0.88, 1.0);
const RED: Color = Color::srgb(0.85, 0.2, 0.25);
const BLUE: Color = Color::srgb(0.15, 0.45, 0.90);
const WELCOME_SECS: f32 = 12.0;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Chip {
    Budget,
    Standing,
}

/// Palette button: which tool it selects (None = the sound toggle) and its hover hint.
#[derive(Component)]
struct PaletteButton(Option<ActiveTool>);

#[derive(Component)]
struct Hint(&'static str);

#[derive(Component)]
struct HintText;
/// The hint pill; collapsed when there is nothing to say.
#[derive(Component)]
struct HintPanel;

/// Equalizer bars of the sound icon; greyed when muted.
#[derive(Component)]
struct SoundBar;

pub fn hint_for(tool: ActiveTool) -> &'static str {
    match tool {
        ActiveTool::Pan => "Drag the map to pan, scroll to zoom.",
        ActiveTool::PlaceStation => "Click the map to place a station.",
        ActiveTool::DrawLine => "Press on a station and drag to another to build a line.",
        ActiveTool::Delete => "Click a station to remove it (right-click works in any tool).",
    }
}

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud).add_systems(Update, (update_chips, update_palette, update_hint));
    }
}

/// Platform UI font on native (clean sans); wasm falls back to Bevy's built-in font.
pub fn ui_font() -> FontSource {
    #[cfg(not(target_arch = "wasm32"))]
    return FontSource::system_ui();
    #[cfg(target_arch = "wasm32")]
    FontSource::default()
}

fn text(s: &str, size: f32) -> impl Bundle {
    (Text::new(s), TextFont { font: ui_font(), font_size: FontSize::Px(size), ..default() }, TextColor(INK))
}

fn chip(commands: &mut Commands, parent: Entity, kind: Chip) {
    let e = commands
        .spawn((
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                border_radius: BorderRadius::all(Val::Px(14.0)),
                ..default()
            },
            BackgroundColor(PANEL),
        ))
        .with_child((kind, text("", 15.0)))
        .id();
    commands.entity(parent).add_child(e);
}

fn button(commands: &mut Commands, parent: Entity, tool: Option<ActiveTool>, label: &str, hint: &'static str) -> Entity {
    let b = commands
        .spawn((
            PaletteButton(tool),
            Hint(hint),
            Hovered::default(),
            Node {
                width: Val::Px(62.0),
                height: Val::Px(62.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(5.0),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(PANEL),
            BorderColor::all(INK),
        ))
        .with_child(text(label, 12.0))
        .id();
    commands.entity(parent).add_child(b);
    b
}

fn bar(left: f32, top: f32, w: f32, h: f32, colour: Color) -> impl Bundle {
    (
        Node { position_type: PositionType::Absolute, left: Val::Px(left), top: Val::Px(top), width: Val::Px(w), height: Val::Px(h), ..default() },
        BackgroundColor(colour),
    )
}

fn dot(left: f32, top: f32, d: f32) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(left),
            top: Val::Px(top),
            width: Val::Px(d),
            height: Val::Px(d),
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(Color::WHITE),
        BorderColor::all(INK),
    )
}

/// Icon slot: a fixed 28x22 box the icon shapes are positioned inside.
fn icon(commands: &mut Commands, button: Entity, shapes: impl FnOnce(&mut ChildSpawnerCommands)) {
    let mut e = commands.spawn(Node { width: Val::Px(28.0), height: Val::Px(22.0), ..default() });
    e.with_children(shapes);
    let id = e.id();
    commands.entity(button).insert_children(0, &[id]);
}

/// Zone colour key (top left): what the green, orange and purple blocks mean.
fn legend(commands: &mut Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(10.0),
                left: Val::Px(10.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                column_gap: Val::Px(12.0),
                border_radius: BorderRadius::all(Val::Px(14.0)),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(PANEL),
        ))
        .with_children(|p| {
            for k in [crate::layout::ZoneKind::Residential, crate::layout::ZoneKind::Commercial, crate::layout::ZoneKind::Industrial] {
                p.spawn(Node { column_gap: Val::Px(5.0), align_items: AlignItems::Center, ..default() }).with_children(|e| {
                    e.spawn((Node { width: Val::Px(12.0), height: Val::Px(12.0), border_radius: BorderRadius::all(Val::Px(3.0)), ..default() }, BackgroundColor(k.colour())));
                    e.spawn(text(k.name(), 13.0));
                });
            }
            p.spawn(Node { column_gap: Val::Px(5.0), align_items: AlignItems::Center, ..default() }).with_children(|e| {
                for c in [crate::roads::tint_colour(4), crate::roads::tint_colour(7)] {
                    e.spawn((Node { width: Val::Px(12.0), height: Val::Px(4.0), ..default() }, BackgroundColor(c)));
                }
                e.spawn(text("Congestion", 13.0));
            });
        });
}

fn spawn_hud(mut commands: Commands) {
    // chips, top right (wraps on narrow windows)
    let chips = commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(10.0),
            right: Val::Px(10.0),
            justify_content: JustifyContent::FlexEnd,
            flex_wrap: FlexWrap::Wrap,
            column_gap: Val::Px(6.0),
            row_gap: Val::Px(6.0),
            ..default()
        })
        .id();
    for k in [Chip::Budget, Chip::Standing] {
        chip(&mut commands, chips, k);
    }
    legend(&mut commands);

    // hint + palette, bottom centre
    let bottom = commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(12.0),
            left: Val::Px(10.0),
            right: Val::Px(10.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(8.0),
            ..default()
        })
        .id();
    let hint = commands
        .spawn((
            HintPanel,
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                max_width: Val::Percent(100.0),
                border_radius: BorderRadius::all(Val::Px(14.0)),
                ..default()
            },
            BackgroundColor(PANEL),
        ))
        .with_child((HintText, text("", 14.0)))
        .id();
    let row = commands
        .spawn(Node { flex_wrap: FlexWrap::Wrap, justify_content: JustifyContent::Center, column_gap: Val::Px(8.0), row_gap: Val::Px(8.0), ..default() })
        .id();
    commands.entity(bottom).add_children(&[hint, row]);

    let pan = button(&mut commands, row, Some(ActiveTool::Pan), "Pan", "Pan tool (1): drag the map, scroll to zoom");
    icon(&mut commands, pan, |p| {
        p.spawn(bar(0.0, 9.0, 28.0, 4.0, INK));
        p.spawn(bar(12.0, 0.0, 4.0, 22.0, INK));
    });
    let st = button(&mut commands, row, Some(ActiveTool::PlaceStation), "Station", "Station tool (2): click the map to place a station");
    icon(&mut commands, st, |p| {
        p.spawn(dot(4.0, 0.0, 22.0));
    });
    let ln = button(&mut commands, row, Some(ActiveTool::DrawLine), "Line", "Line tool (3): drag from one station to another");
    icon(&mut commands, ln, |p| {
        p.spawn(bar(4.0, 8.0, 20.0, 6.0, BLUE));
        p.spawn(dot(0.0, 5.0, 12.0));
        p.spawn(dot(16.0, 5.0, 12.0));
    });
    let del = button(&mut commands, row, Some(ActiveTool::Delete), "Delete", "Delete tool (4): click a station to remove it, or right-click any station");
    icon(&mut commands, del, |p| {
        p.spawn((bar(2.0, 9.0, 24.0, 4.0, RED), Transform::from_rotation(Quat::from_rotation_z(0.785))));
        p.spawn((bar(2.0, 9.0, 24.0, 4.0, RED), Transform::from_rotation(Quat::from_rotation_z(-0.785))));
    });
    let snd = button(&mut commands, row, None, "Sound", "Sound: click to mute or unmute");
    icon(&mut commands, snd, |p| {
        for (i, h) in [8.0, 16.0, 22.0, 12.0].into_iter().enumerate() {
            p.spawn((SoundBar, bar(i as f32 * 7.0, 22.0 - h, 4.0, h, INK)));
        }
    });
}

fn update_chips(eco: Res<Economy>, budget: Res<CurrentBudget>, mut q: Query<(&Chip, &mut Text)>) {
    let (amount, spent, standing, _reserve) = budget_summary(&eco, &budget);
    let z = |v: f64| v + 0.0; // normalise -0.0 so it never prints "-0"
    let (amount, _spent, standing) = (z(amount), z(spent), z(standing));
    for (k, mut t) in &mut q {
        t.0 = match k {
            Chip::Budget => format!("Budget {amount:.0}"),
            Chip::Standing => format!("Standing {:.0}%", standing * 100.0),
        };
    }
}

fn update_palette(
    mouse: Res<ButtonInput<MouseButton>>,
    mut tool: ResMut<ActiveTool>,
    mut sound: ResMut<SoundSettings>,
    mut buttons: Query<(&PaletteButton, &Hovered, &mut BackgroundColor, &mut BorderColor)>,
    mut bars: Query<&mut BackgroundColor, (With<SoundBar>, Without<PaletteButton>)>,
) {
    let clicked = mouse.just_released(MouseButton::Left);
    for (b, hov, mut bg, mut border) in &mut buttons {
        if clicked && hov.get() {
            match b.0 {
                Some(t) => *tool = t,
                None => sound.muted = !sound.muted,
            }
        }
        let selected = b.0 == Some(*tool);
        bg.0 = if selected { SELECTED } else if hov.get() { Color::srgb(0.93, 0.95, 1.0) } else { PANEL };
        *border = BorderColor::all(if selected { BLUE } else { INK });
    }
    for mut bar in &mut bars {
        bar.0 = if sound.muted { Color::srgb(0.7, 0.7, 0.72) } else { INK };
    }
}

fn update_hint(
    time: Res<Time>,
    tool: Res<ActiveTool>,
    buttons: Query<(&Hovered, &Hint)>,
    extend: Res<LineExtend>,
    cam: Option<Res<crate::camera::MapCam>>,
    mut start: Local<Option<Vec2>>,
    mut panned: Local<bool>,
    mut panel: Query<&mut Node, With<HintPanel>>,
    mut q: Query<&mut Text, With<HintText>>,
) {
    if let Some(c) = cam {
        if *start.get_or_insert(c.focus) != c.focus {
            *panned = true;
        }
    }
    let line = if extend.0.is_some() {
        "Click where the line should extend to. Esc or right-click cancels."
    } else if let Some((_, h)) = buttons.iter().find(|(h, _)| h.get()) {
        h.0
    } else if *panned && *tool == ActiveTool::Pan {
        "" // they know how to pan: no more pan hint
    } else if time.elapsed_secs() < WELCOME_SECS && *tool == ActiveTool::Pan {
        "Welcome. Pick Station and click the map twice, then Line and drag between them."
    } else {
        hint_for(*tool)
    };
    for mut n in &mut panel {
        let d = if line.is_empty() { Display::None } else { Display::Flex };
        if n.display != d {
            n.display = d;
        }
    }
    for mut t in &mut q {
        if t.0 != line {
            t.0 = line.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_distinct_hint() {
        let h: Vec<_> = [ActiveTool::Pan, ActiveTool::PlaceStation, ActiveTool::DrawLine, ActiveTool::Delete].map(hint_for).to_vec();
        assert!(h.iter().all(|s| !s.is_empty() && s.len() < 80), "one-line hints");
        assert!(h[0] != h[1] && h[1] != h[2]);
    }
}
