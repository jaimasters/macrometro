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
        .init_resource::<Window5>()
        .add_systems(First, frame_start)
        .add_systems(Last, worst_frame)
        .run();
}

/// Averages hide hitches. Per 5 s window: worst frame, frames over 20 ms, and the worst main-world CPU time
/// (First..Last) so a slow frame can be told apart as sim/ECS-bound (cpu ~ frame) or render/GPU-bound (cpu small),
/// plus the camera height at the worst frame.
#[derive(Resource, Default)]
struct Window5 {
    start: Option<std::time::Instant>,
    worst: f32,
    worst_view: f32,
    slow: u32,
    frames: u32,
    cpu_max: f32,
    secs: f32,
}

fn frame_start(mut w: ResMut<Window5>) {
    w.start = Some(std::time::Instant::now());
}

fn worst_frame(time: Res<Time<Real>>, cam: Option<Res<app::MapCam>>, mut w: ResMut<Window5>) {
    let ms = time.delta_secs() * 1000.0;
    if ms > w.worst {
        w.worst = ms;
        w.worst_view = cam.map_or(0.0, |c| c.view_h);
    }
    w.slow += (ms > 20.0) as u32;
    w.frames += 1;
    w.secs += time.delta_secs();
    if let Some(s) = w.start {
        w.cpu_max = w.cpu_max.max(s.elapsed().as_secs_f32() * 1000.0);
    }
    if w.secs >= 5.0 {
        info!(
            "worst_frame {:.1} ms (view_h {:.0}), {} of {} frames over 20 ms, main-world cpu max {:.1} ms",
            w.worst, w.worst_view, w.slow, w.frames, w.cpu_max
        );
        *w = Window5::default();
    }
}
