//! macrometro entry point: all plugins composed by `app`; native and wasm share it.
//! `MM_WINDOW=WxH` overrides the window size (used to check small windows); `MM_SHOT=file.png` saves a scripted frame.
use bevy::prelude::*;
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
        ))
        .run();
}
