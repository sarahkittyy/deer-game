//! The world is a 512 m square that wraps. Everything with a [`Wrapped`] position is drawn at the
//! copy of itself nearest the [`Focus`] (the player's deer), so the world looks endless and the
//! edges never show. The focus itself never wraps; it walks on forever in plain coordinates.

use bevy::prelude::*;
use deer_shared::world::wrap_delta;

/// The point the world is centred on, in unwrapped coordinates.
#[derive(Resource, Default, Clone, Copy)]
pub struct Focus(pub Vec3);

/// A position in wrapped world coordinates (x, z in `[0, WORLD_SIZE)`). The entity's
/// `Transform` translation is rewritten each frame to the nearest copy of it.
#[derive(Component, Clone, Copy)]
pub struct Wrapped(pub Vec3);

/// The copy of wrapped position `p` nearest to `focus`.
pub fn nearest(p: Vec3, focus: Vec3) -> Vec3 {
    Vec3::new(
        focus.x + wrap_delta(p.x - focus.x),
        p.y,
        focus.z + wrap_delta(p.z - focus.z),
    )
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Focus>().add_systems(
        PostUpdate,
        place_wrapped.before(TransformSystems::Propagate),
    );
}

fn place_wrapped(focus: Res<Focus>, mut q: Query<(&Wrapped, &mut Transform)>) {
    for (w, mut t) in &mut q {
        let p = nearest(w.0, focus.0);
        // Only touch the transform when it moves, so static chunks are not re-propagated.
        if t.translation.distance_squared(p) > 1e-6 {
            t.translation = p;
        }
    }
}
