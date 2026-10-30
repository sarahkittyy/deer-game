//! The lake surface material.

use bevy::{
    image::{
        ImageAddressMode, ImageFilterMode, ImageLoaderSettings, ImageSampler,
        ImageSamplerDescriptor,
    },
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};

pub type Water = ExtendedMaterial<StandardMaterial, WaterExt>;

#[derive(Resource)]
pub struct WaterMaterial(pub Handle<Water>);

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct WaterExt {
    #[texture(100)]
    #[sampler(101)]
    normals: Handle<Image>,
    #[uniform(102)]
    settings: WaterSettings,
}

#[derive(ShaderType, Debug, Clone)]
struct WaterSettings {
    /// Scroll speed of each octave in uv per second, two octaves per `vec4`.
    octave_vectors: [Vec4; 2],
    /// Tiles of each octave across the world. Whole numbers, so the ripples wrap with the world.
    octave_scales: Vec4,
    octave_strengths: Vec4,
}

impl MaterialExtension for WaterExt {
    /// The WebGL2 build lights everything in the forward pass.
    fn fragment_shader() -> ShaderRef {
        "shaders/water_forward.wesl".into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        "shaders/water.wesl".into()
    }
}

pub fn plugin(app: &mut App) {
    app.add_plugins(MaterialPlugin::<Water>::default())
        .add_systems(PreStartup, create);
}

fn create(mut commands: Commands, assets: Res<AssetServer>, mut materials: ResMut<Assets<Water>>) {
    let normals = assets
        .load_builder()
        .with_settings(|s: &mut ImageLoaderSettings| {
            s.is_srgb = false;
            s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                ..default()
            });
        })
        .load("textures/water_normals.png");
    let handle = materials.add(Water {
        base: StandardMaterial {
            // Dark peaty water: what shows through where nothing is reflected.
            base_color: Color::srgb(0.035, 0.05, 0.045),
            perceptual_roughness: 0.02,
            reflectance: 0.6,
            ..default()
        },
        extension: WaterExt {
            normals,
            settings: WaterSettings {
                octave_vectors: [
                    Vec4::new(0.0040, 0.0030, 0.0036, -0.0031),
                    Vec4::new(0.0077, 0.0069, -0.0074, -0.0098),
                ],
                octave_scales: Vec4::new(64.0, 131.0, 412.0, 779.0),
                octave_strengths: Vec4::new(0.16, 0.18, 0.093, 0.044) * 0.25,
            },
        },
    });
    commands.insert_resource(WaterMaterial(handle));
}
