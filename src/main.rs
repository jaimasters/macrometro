//! macrometro entry point: all plugins composed by `app`; native and wasm share it.
//! `MM_WINDOW=WxH` overrides the window size (used to check small windows); `MM_SHOT=file.png` saves a scripted frame.
use bevy::diagnostic::{EntityCountDiagnosticsPlugin, FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin};
use bevy::prelude::*;
use std::time::Duration;
use bevy::window::WindowResolution;

fn main() {
    let (w, h) = std::env::var("MM_WINDOW")
        .ok()
        .and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))))
        .unwrap_or((1280, 720));
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window { title: "macrometro".into(), resolution: WindowResolution::new(w, h), ..default() }),
                ..default()
            }),
            app::MacrometroPlugins,
            // Live perf: frame time / fps / entity count to stdout (.game.log) every 5 s.
            FrameTimeDiagnosticsPlugin::default(),
            EntityCountDiagnosticsPlugin::default(),
            LogDiagnosticsPlugin { wait_duration: Duration::from_secs(5), ..default() },
        ))
        .run();
}
