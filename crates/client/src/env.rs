//! Dawn: a low sun through a physical atmosphere, morning mist that catches light shafts between
//! the trees, and the camera's post-processing.

use bevy::{
    anti_alias::{fxaa::Fxaa, taa::TemporalAntiAliasing},
    camera::Exposure,
    core_pipeline::{
        prepass::{DeferredPrepass, DepthPrepass},
        tonemapping::Tonemapping,
    },
    light::{
        Atmosphere, AtmosphereEnvironmentMapLight, CascadeShadowConfigBuilder, EnvironmentMapLight,
        SunDisk, atmosphere::ScatteringMedium, light_consts::lux,
    },
    asset::RenderAssetUsages,
    pbr::{AtmosphereSettings, ScreenSpaceReflections},
    post_process::bloom::Bloom,
    prelude::*,
    render::render_resource::{
        Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
    },
};

/// The one 3D camera; `camera.rs` moves it.
#[derive(Component)]
pub struct MainCamera;

/// Sun elevation above the horizon, in degrees. Just risen.
/// High enough for light to fall through gaps in the canopy, low enough to stay golden.
const SUN_ELEVATION: f32 = 50.0;
/// Compass direction the sun shines *from*, in degrees.
const SUN_AZIMUTH: f32 = 70.0;

/// The WebGL2 build has no compute shaders (so no physical atmosphere or the light it gives),
/// no screen-space reflections and no TAA: a plain sky colour and a made-up sky light stand in.
/// `DEER_LITE=1` uses the same renderer natively, to look at it without a browser.
pub fn lite() -> bool {
    cfg!(feature = "webgl2") || std::env::var("DEER_LITE").is_ok()
}

pub fn plugin(app: &mut App) {
    app.insert_resource(GlobalAmbientLight::NONE)
        .insert_resource(ClearColor(if lite() {
            Color::srgb(0.62, 0.72, 0.86)
        } else {
            Color::BLACK
        }))
        .add_systems(Startup, setup)
        .add_systems(Update, freeze_environment_light);
}

pub fn sun_direction() -> Vec3 {
    let (e, a) = (SUN_ELEVATION.to_radians(), SUN_AZIMUTH.to_radians());
    // Direction *towards* the sun.
    Vec3::new(e.cos() * a.cos(), e.sin(), e.cos() * a.sin())
}

fn setup(
    mut commands: Commands,
    mut media: ResMut<Assets<ScatteringMedium>>,
    mut images: ResMut<Assets<Image>>,
) {
    commands.insert_resource(bevy::light::DirectionalLightShadowMap {
        // WebGL2's single cascade spreads over the whole 50 m, so it gets the two cascades' texels.
        size: crate::debug::shadow_size().unwrap_or(if lite() { 2048 } else { 1024 }),
    });
    let lite = lite();
    if !lite && !crate::debug::off("atmo") {
        commands.spawn(Atmosphere::earth(
            media.add(ScatteringMedium::earth(256, 256)),
        ));
    }

    commands
        .spawn((
            DirectionalLight {
                shadow_maps_enabled: !crate::debug::off("shadows"),
                // The atmosphere dims raw sunlight on its way down; without it, start dimmer.
                illuminance: if lite {
                    lux::DIRECT_SUNLIGHT
                } else {
                    lux::RAW_SUNLIGHT
                },
                // Morning gold, warmer than the atmosphere alone makes it at this height.
                color: Color::srgb(1.0, 0.86, 0.66),
                ..default()
            },
            Transform::from_translation(sun_direction()).looking_at(Vec3::ZERO, Vec3::Y),
            {
                // Two cascades at 1024 px: shadows are mostly overlapping alpha-tested leaf cards, so their
                // cost is fill rate, and this halves the frame time compared to 3 x 2048 px.
                // Bevy on WebGL2 keeps only the first cascade and drops the rest, so there the one
                // cascade has to reach all the way, or shadows stop at its far edge.
                let c = crate::debug::cascades().unwrap_or(if lite {
                    (1, 50.0, 50.0)
                } else {
                    (2, 14.0, 50.0)
                });
                CascadeShadowConfigBuilder {
                    num_cascades: c.0,
                    first_cascade_far_bound: c.1,
                    maximum_distance: c.2,
                    ..default()
                }
                .build()
            },
        ))
        .insert_if(SunDisk::EARTH, || !lite);
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
                far: if lite { 150.0 } else { 215.0 },
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
    if !lite && !off("atmo") {
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
    if lite {
        let sky = images.add(lite_sky_light());
        cam.insert(EnvironmentMapLight {
            diffuse_map: sky.clone(),
            specular_map: sky,
            intensity: 2600.0,
            rotation: Quat::IDENTITY,
            affects_lightmapped_mesh_diffuse: true,
        });
    }
    if !off("bloom") {
        cam.insert(Bloom {
            max_mip_dimension: crate::debug::knob("DEER_BLOOM_MIP").unwrap_or(512.0) as u32,
            ..Bloom::NATURAL
        });
    }
    if lite {
        cam.insert(Fxaa::default());
    } else if !off("taa") {
        cam.insert(TemporalAntiAliasing::default());
    }
    if !lite && !off("ssr") {
        cam.insert(ScreenSpaceReflections {
            min_perceptual_roughness: 0.0..0.0,
            max_perceptual_roughness: 0.15..0.2,
            ..default()
        });
    } else if !lite {
        // Reflections bring these with them, and the materials are deferred with or without.
        cam.insert((DepthPrepass, DeferredPrepass));
    }
    if !off("fog") {
        cam.insert(DistanceFog {
            color: Color::srgba(0.62, 0.62, 0.72, 1.0),
            directional_light_color: Color::srgba(1.0, 0.72, 0.45, 0.6),
            directional_light_exponent: 12.0,
            // WebGL2 pays for every draw on the CPU, so its fog closes in sooner and hides half
            // the forest's trees behind the far plane.
            falloff: FogFalloff::from_visibility_colors(
                if lite { 132.0 } else { 190.0 },
                Color::srgb(0.56, 0.60, 0.68),
                Color::srgb(0.88, 0.82, 0.78),
            ),
        });
    }
}

/// WebGL2's stand-in for the atmosphere's environment map: blue from above, dark earth from below
/// and a warm side towards the sun, so that things in the shade still show their shape. It is so
/// smooth that one small image does for both diffuse and specular.
fn lite_sky_light() -> Image {
    const SIZE: u32 = 16;
    let sky = Vec3::new(0.86, 0.94, 1.0);
    let earth = Vec3::new(0.2, 0.17, 0.12);
    let warm = Vec3::new(0.55, 0.4, 0.22);
    let sun = sun_direction();
    // Cubemap faces in +X, -X, +Y, -Y, +Z, -Z order: which way each looks, given the texel's
    // position (u, v) across it.
    let faces: [fn(f32, f32) -> Vec3; 6] = [
        |u, v| Vec3::new(1.0, -v, -u),
        |u, v| Vec3::new(-1.0, -v, u),
        |u, v| Vec3::new(u, 1.0, v),
        |u, v| Vec3::new(u, -1.0, -v),
        |u, v| Vec3::new(u, -v, 1.0),
        |u, v| Vec3::new(-u, -v, -1.0),
    ];
    let mut data = Vec::with_capacity((SIZE * SIZE * 6 * 4) as usize);
    for face in faces {
        for y in 0..SIZE {
            for x in 0..SIZE {
                let at = |i: u32| (i as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
                let mut dir = face(at(x), at(y)).normalize();
                // Bevy looks up its cubemaps with z flipped.
                dir.z = -dir.z;
                let up = (dir.y * 0.5 + 0.5).clamp(0.0, 1.0);
                let toward_sun = dir.dot(sun).max(0.0);
                let c = earth.lerp(sky, up * up * (3.0 - 2.0 * up))
                    + warm * toward_sun * toward_sun;
                let c = LinearRgba::rgb(c.x.min(1.0), c.y.min(1.0), c.z.min(1.0));
                data.extend_from_slice(&Color::from(c).to_srgba().to_u8_array());
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
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
