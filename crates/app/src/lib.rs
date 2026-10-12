//! Composes all feature plugins into the first playable shell. Spec: docs/features/app/SPEC.md.
use bevy::app::PluginGroupBuilder;
use bevy::prelude::*;
use std::sync::Arc;

mod camera;
mod layout;
mod capture;
mod hud;
pub mod shell;
mod terrain;
mod zones;

pub use camera::{MapCam, MapCameraPlugin};
pub use shell::{to_transit, ShellPlugin, StationMap};
pub use terrain::{chunk_triangles, ground_y, wanted_chunks, TerrainPlugin};
pub use zones::{build_layout, GrowthLink, GrowthSimulation, GrowthSnapshot, ZoneBlock, ZoneGeo, ZoneKind, ZoneLayout, ZonesPlugin};

/// Every macrometro feature plugin, in dependency order, then the shell that ties them together.
pub struct MacrometroPlugins;

impl PluginGroup for MacrometroPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(sim_core::SimCorePlugin)
            .add(world_gen::WorldGenPlugin)
            .add(transit::TransitPlugin)
            .add(growth::GrowthPlugin)
            .add(economy::EconomyPlugin)
            .add(render_budget::RenderBudgetPlugin)
            .add(save::SavePlugin)
            .add(ui_tools::UiToolsPlugin)
            .add(ShellSetup)
            .add(TerrainPlugin)
            .add(MapCameraPlugin)
            .add(ZonesPlugin)
            .add(ShellPlugin)
            .add(sound::SoundPlugin)
            .add(hud::HudPlugin)
            .add(capture::CapturePlugin)
    }
}

/// Seeds the ground-height lookup and frames the opening camera on the first city.
struct ShellSetup;

impl Plugin for ShellSetup {
    fn build(&self, app: &mut App) {
        let seed = app.world().get_resource::<world_gen::WorldSeed>().map_or(0, |s| s.0);
        app.insert_resource(ui_tools::GroundHeight(Arc::new(move |x, z| ground_y(seed, x, z))));
        if let Some(c) = app.world().get_resource::<world_gen::Cities>().and_then(|c| c.0.first()) {
            let (ox, oz) = c.center.origin_cell();
            let half = world_gen::CHUNK_SIZE as f32 / 2.0;
            app.insert_resource(MapCam::framing(Vec2::new(ox as f32 + half, oz as f32 + half), 190.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};

    #[test]
    fn shell_boots_headless_and_zones_grow() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            .add_plugins(bevy::audio::AudioPlugin::default())
            .add_plugins(MacrometroPlugins);
        app.update();
        let n = app.world().resource::<ZoneLayout>().0.len();
        assert!(n > 0);
        let zones: std::collections::HashSet<usize> = app.world_mut().query::<&ZoneBlock>().iter(app.world()).map(|b| b.0).collect();
        assert_eq!(zones.len(), n);
        // Transit publishes costs on FixedUpdate; the sim thread then grows zones.
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().resource::<growth::Costs>().0.is_some(), "transit never published costs");
        let start: f32 = app.world().resource::<GrowthLink>().snapshots.latest().pop.iter().sum();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            app.update();
            let now: f32 = app.world().resource::<GrowthLink>().snapshots.latest().pop.iter().sum();
            if now > start {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "zones never grew");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    #[test]
    fn drawn_line_reaches_transit_and_spends_budget() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            .add_plugins(bevy::audio::AudioPlugin::default())
            .add_plugins(MacrometroPlugins);
        app.update();
        app.world_mut().run_schedule(FixedUpdate);
        let centre = app.world().resource::<ZoneLayout>().0[0].center.as_ivec2();
        let cells = vec![centre, centre + IVec2::new(30, 0), centre + IVec2::new(60, 10)];
        app.world_mut().write_message(ui_tools::ToolCommand {
            agency: shell::PLAYER,
            action: ui_tools::ToolAction::DrawLine { cells },
        });
        app.update();
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().resource::<transit::TransitErrors>().0.is_empty(), "{:?}", app.world().resource::<transit::TransitErrors>().0);
        assert_eq!(app.world().resource::<transit::TransitNet>().0.lines.len(), 1);
        let spent: f64 = app.world().resource::<economy::Economy>().0.agencies[&shell::PLAYER].spend.values().sum();
        assert!(spent > 0.0, "construction spend not recorded");
    }
}
