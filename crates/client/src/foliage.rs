//! Leaves and grass that sway in the wind: a `StandardMaterial` with a vertex shader on top.

use bevy::{
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};

pub type Foliage = ExtendedMaterial<StandardMaterial, WindExt>;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct WindExt {
    #[uniform(100)]
    pub wind: WindUniform,
}

#[derive(ShaderType, Debug, Clone, Copy)]
pub struct WindUniform {
    /// x: sway amplitude (m), y: flutter amplitude (m), z: 1 / model height.
    pub params: Vec4,
}

/// How a kind of plant moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sway {
    pub amplitude: f32,
    pub flutter: f32,
    /// Height of the model in its own units: the tip moves the full amplitude.
    pub height: f32,
}

impl Sway {
    pub const TREE: Sway = Sway {
        amplitude: 0.22,
        flutter: 0.035,
        height: 9.0,
    };
    // Barely moving: berries sit on these leaves and do not sway with them.
    pub const BUSH: Sway = Sway {
        amplitude: 0.012,
        flutter: 0.004,
        height: 1.6,
    };
    pub const GRASS: Sway = Sway {
        amplitude: 0.12,
        flutter: 0.02,
        height: 1.3,
    };
    pub const REED: Sway = Sway {
        amplitude: 0.1,
        flutter: 0.015,
        height: 1.6,
    };
}

impl MaterialExtension for WindExt {
    fn vertex_shader() -> ShaderRef {
        "shaders/foliage.wesl".into()
    }
    fn prepass_vertex_shader() -> ShaderRef {
        "shaders/foliage_prepass.wesl".into()
    }
    fn deferred_vertex_shader() -> ShaderRef {
        "shaders/foliage_prepass.wesl".into()
    }
}

pub fn make(base: StandardMaterial, sway: Sway) -> Foliage {
    Foliage {
        base,
        extension: WindExt {
            wind: WindUniform {
                params: Vec4::new(sway.amplitude, sway.flutter, 1.0 / sway.height, 0.0),
            },
        },
    }
}

pub fn plugin(app: &mut App) {
    app.add_plugins(MaterialPlugin::<Foliage>::default());
}
