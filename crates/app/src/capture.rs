//! `MM_SHOT=path.png`: frame the city, build a scripted line, run ~5 s, save the frame, exit.
//! Verification aid (screencapture does not work over ssh); inert when the variable is unset.
use crate::shell::PLAYER;
use crate::zones::ZoneLayout;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use ui_tools::{snap, ActiveTool, ToolAction, ToolCommand};

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        if let Ok(path) = std::env::var("MM_SHOT") {
            app.insert_resource(ShotPath(path)).add_systems(Update, script);
        }
    }
}

#[derive(Resource)]
struct ShotPath(String);

fn script(
    time: Res<Time>,
    path: Res<ShotPath>,
    layout: Res<ZoneLayout>,
    mut tool: ResMut<ActiveTool>,
    mut out: MessageWriter<ToolCommand>,
    mut exit: MessageWriter<AppExit>,
    mut stage: Local<u8>,
    mut commands: Commands,
) {
    let t = time.elapsed_secs();
    let c = layout.0.first().map_or(Vec2::ZERO, |g| g.center);
    let cell = |dx: f32, dz: f32| snap(c + Vec2::new(dx, dz));
    if *stage == 0 && t > 1.5 {
        *stage = 1;
        *tool = ActiveTool::DrawLine;
        for cells in [vec![cell(-56.0, 24.0), cell(40.0, -28.0)], vec![cell(-56.0, 24.0), cell(8.0, 64.0)]] {
            out.write(ToolCommand { agency: PLAYER, action: ToolAction::DrawLine { cells } });
        }
    } else if *stage == 1 && t > 6.5 {
        *stage = 2;
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.0.clone()));
    } else if *stage == 2 && t > 9.0 {
        exit.write(AppExit::Success);
    }
}
