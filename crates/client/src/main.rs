//! deer game: a deer wandering a wrapping low-poly forest at dawn, alongside everyone else's deer.

mod ambient;
mod audio;
mod bushes;
mod debug;
mod deer;
mod env;
mod foliage;
mod ghosts;
mod interact;
mod lod;
mod net;
mod player;
mod render_scale;
mod ripples;
mod shafts;
mod terrain;
mod ui;
mod vegetation;
mod water;
#[cfg(target_arch = "wasm32")]
mod web;
mod wrap;

use bevy::{asset::AssetMetaCheck, pbr::DefaultOpaqueRendererMethod, prelude::*};
use deer_shared::world::{WORLD_SEED, World};
use std::sync::Arc;

/// The world everyone shares, generated from the build's seed.
#[derive(Resource, Clone)]
pub struct GameWorld(pub Arc<World>);

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    Title,
    Playing,
}

/// Despawned when leaving the game for the title screen.
#[derive(Component)]
pub struct GameEntity;

fn main() {
    let mut app = App::new();
    app
        // Deferred lighting: the lakes' screen-space reflections need it.
        .insert_resource(DefaultOpaqueRendererMethod::deferred())
        .insert_resource(GameWorld(Arc::new(World::generate(WORLD_SEED))))
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "deer game".into(),
                        canvas: Some("#deer-canvas".into()),
                        fit_canvas_to_parent: true,
                        prevent_default_event_handling: true,
                        // Uncapped while profiling, so frame times are not just vsync.
                        // Hidden for debug screenshots and profiling, which render off-screen
                        // (and so a test run never pops up over whatever you are doing).
                        visible: std::env::var("DEER_PROFILE").is_err()
                            && std::env::var("DEER_SHOT").is_err(),
                        present_mode: if std::env::var("DEER_PROFILE").is_ok() {
                            bevy::window::PresentMode::Immediate
                        } else {
                            bevy::window::PresentMode::AutoVsync
                        },
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    // The web build serves assets from a content-hashed directory (build-web.sh).
                    file_path: option_env!("DEER_ASSET_DIR").unwrap_or("assets").into(),
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                })
                .set(audio::audio_plugin()),
        )
        .init_state::<AppState>()
        .add_plugins((
            wrap::plugin,
            water::plugin,
            terrain::plugin,
            env::plugin,
            foliage::plugin,
            vegetation::plugin,
            deer::plugin,
            player::plugin,
            debug::plugin,
        ))
        .add_plugins((
            audio::plugin,
            net::plugin,
            ghosts::plugin,
            bushes::plugin,
            interact::plugin,
            ui::plugin,
            ambient::plugin,
            shafts::plugin,
            render_scale::plugin,
            ripples::plugin,
        ));
    #[cfg(target_arch = "wasm32")]
    app.add_plugins(web::plugin);
    app.run();
}
