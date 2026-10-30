//! Dawn: a low sun through a physical atmosphere, morning mist that catches light shafts between
//! the trees, and the camera's post-processing.

use bevy::{
    anti_alias::taa::TemporalAntiAliasing,
    camera::Exposure,
    core_pipeline::tonemapping::Tonemapping,
    light::{
        Atmosphere, AtmosphereEnvironmentMapLight, CascadeShadowConfigBuilder, EnvironmentMapLight,
        SunDisk, atmosphere::ScatteringMedium, light_consts::lux,
    },
    pbr::{AtmosphereSettings, ScreenSpaceReflections},
    post_process::bloom::Bloom,
    prelude::*,
};

/// The one 3D camera; `camera.rs` moves it.
#[derive(Component)]
pub struct MainCamera;

/// Sun elevation above the horizon, in degrees. Just risen.
/// High enough for light to fall through gaps in the canopy, low enough to stay golden.
const SUN_ELEVATION: f32 = 50.0;
/// Compass direction the sun shines *from*, in degrees.
const SUN_AZIMUTH: f32 = 70.0;

pub fn plugin(app: &mut App) {
    app.insert_resource(GlobalAmbientLight::NONE)
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, setup)
        .add_systems(Update, freeze_environment_light);
}

pub fn sun_direction() -> Vec3 {
    let (e, a) = (SUN_ELEVATION.to_radians(), SUN_AZIMUTH.to_radians());
    // Direction *towards* the sun.
    Vec3::new(e.cos() * a.cos(), e.sin(), e.cos() * a.sin())
}

fn setup(mut commands: Commands, mut media: ResMut<Assets<ScatteringMedium>>) {
    commands.insert_resource(bevy::light::DirectionalLightShadowMap {
        size: crate::debug::shadow_size().unwrap_or(1024),
    });
    if !crate::debug::off("atmo") {
        commands.spawn(Atmosphere::earth(
            media.add(ScatteringMedium::earth(256, 256)),
        ));
    }

    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: !crate::debug::off("shadows"),
            illuminance: lux::RAW_SUNLIGHT,
            // Morning gold, warmer than the atmosphere alone makes it at this height.
            color: Color::srgb(1.0, 0.86, 0.66),
            ..default()
        },
        Transform::from_translation(sun_direction()).looking_at(Vec3::ZERO, Vec3::Y),
        {
            // Two cascades at 1024 px: shadows are mostly overlapping alpha-tested leaf cards, so their
            // cost is fill rate, and this halves the frame time compared to 3 x 2048 px.
            let c = crate::debug::cascades().unwrap_or((2, 14.0, 50.0));
            CascadeShadowConfigBuilder {
                num_cascades: c.0,
                first_cascade_far_bound: c.1,
                maximum_distance: c.2,
                ..default()
            }
            .build()
        },
        SunDisk::EARTH,
    ));
    if std::env::var("DEER_SHADOW_OC").is_ok() {
        commands.run_system_cached(
            |mut c: Commands, q: Query<Entity, With<DirectionalLight>>| {
                for e in &q {
                    c.entity(e)
                        .insert(bevy::render::occlusion_culling::OcclusionCulling);
                }
            },
        );
    }

    let off = crate::debug::off;
    let cam = commands
        .spawn((
            MainCamera,
            Camera3d::default(),
            // UI follows this camera even when it renders off-screen (debug screenshots).
            IsDefaultUiCamera,
            Projection::Perspective(PerspectiveProjection {
                fov: 70f32.to_radians(),
                near: 0.05,
                // Just past where the fog turns opaque.
                far: 215.0,
                ..default()
            }),
            Transform::from_xyz(40.0, 8.0, 40.0).looking_at(Vec3::new(60.0, 4.0, 60.0), Vec3::Y),
            Exposure { ev100: 12.4 },
            Tonemapping::AgX,
            Msaa::Off,
        ))
        .id();
    if std::env::var("DEER_HW2X2").is_ok() {
        commands
            .entity(cam)
            .insert(bevy::light::ShadowFilteringMethod::Hardware2x2);
    }
    let mut cam = commands.entity(cam);
    if !off("atmo") {
        // The canopy hides most of the sky, so less light comes from it.
        cam.insert(AtmosphereSettings::default());
        if !off("envlight") {
            let size = crate::debug::knob("DEER_ENV_SIZE").unwrap_or(128.0) as u32;
            cam.insert(AtmosphereEnvironmentMapLight {
                intensity: 0.55,
                size: UVec2::splat(size),
                ..default()
            });
        }
    }
    if !off("bloom") {
        cam.insert(Bloom {
            max_mip_dimension: crate::debug::knob("DEER_BLOOM_MIP").unwrap_or(512.0) as u32,
            ..Bloom::NATURAL
        });
    }
    if !off("taa") {
        cam.insert(TemporalAntiAliasing::default());
    }
    if !off("ssr") {
        cam.insert(ScreenSpaceReflections {
            min_perceptual_roughness: 0.0..0.0,
            max_perceptual_roughness: 0.15..0.2,
            ..default()
        });
    }
    if !off("fog") {
        cam.insert(DistanceFog {
            color: Color::srgba(0.62, 0.62, 0.72, 1.0),
            directional_light_color: Color::srgba(1.0, 0.72, 0.45, 0.6),
            directional_light_exponent: 12.0,
            falloff: FogFalloff::from_visibility_colors(
                190.0,
                Color::srgb(0.56, 0.60, 0.68),
                Color::srgb(0.88, 0.82, 0.78),
            ),
        });
    }
}

/// The sky never changes, but Bevy re-renders and re-filters the atmosphere's environment map
/// every frame (several milliseconds). Once it has been made, keep the result as a plain
/// environment map and stop generating it.
fn freeze_environment_light(
    mut commands: Commands,
    time: Res<Time>,
    cam: Query<
        (Entity, Option<&EnvironmentMapLight>),
        (With<MainCamera>, With<AtmosphereEnvironmentMapLight>),
    >,
    mut ready_since: Local<Option<f32>>,
    mut saved: Local<Option<(Entity, EnvironmentMapLight)>>,
) {
    // Second step, a frame later: put the kept maps back (removing the generator removes them).
    if let Some((e, light)) = saved.take() {
        commands.entity(e).insert(light);
        return;
    }
    let Ok((e, Some(light))) = cam.single() else {
        return;
    };
    let now = time.elapsed_secs();
    // Give the generation pipelines time to compile and run (longer on the web).
    let since = *ready_since.get_or_insert(now);
    if now - since < 5.0 {
        return;
    }
    info!("environment light made; keeping it and stopping its generation");
    *saved = Some((e, light.clone()));
    commands.entity(e).remove::<AtmosphereEnvironmentMapLight>();
}
