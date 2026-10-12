//! macrometro entry point: all plugins composed by `app`; native and wasm share it.
use bevy::prelude::*;

fn main() {
    App::new().add_plugins((DefaultPlugins, app::MacrometroPlugins)).run();
}
