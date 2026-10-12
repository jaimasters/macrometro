//! Composes all feature plugins. Spec: docs/features/app/SPEC.md.
use bevy::app::PluginGroupBuilder;
use bevy::prelude::*;

/// Every macrometro feature plugin, in dependency order.
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
    }
}
