//! Everything both the game and the server need: the seeded world and the wire protocol.

pub mod noise;
pub mod protocol;
pub mod world;

#[cfg(test)]
mod tests {
    use crate::world::*;

    #[test]
    fn world_stats() {
        let w = World::generate(1);
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        let mut water = 0;
        let n = 512;
        for j in 0..n {
            for i in 0..n {
                let h = w.height(i as f32, j as f32);
                min = min.min(h);
                max = max.max(h);
                if w.is_water(i as f32, j as f32) {
                    water += 1;
                }
            }
        }
        println!(
            "min {min} max {max} water {:.1}% lakes {} trees {} boulders {} bushes {}",
            water as f32 / (n * n) as f32 * 100.0,
            w.lakes.len(),
            w.trees.len(),
            w.boulders.len(),
            w.bush_slots.len()
        );
        // Wraps seamlessly.
        assert!((w.height(0.3, 7.0) - w.height(WORLD_SIZE + 0.3, 7.0)).abs() < 1e-4);
    }

    #[test]
    fn no_water_outside_lakes() {
        let w = World::generate(7);
        for j in 0..256 {
            for i in 0..256 {
                let (x, z) = (i as f32 * 2.0, j as f32 * 2.0);
                if w.is_water(x, z) {
                    assert!(w.lake_distance(x, z) < 12.0, "stray water at {x},{z}");
                }
            }
        }
    }
}

#[cfg(test)]
mod dump {
    #[test]
    #[ignore]
    fn lakes() {
        let w = crate::world::World::generate(crate::world::WORLD_SEED);
        for l in &w.lakes {
            println!("lake {:?} r {}", l.center, l.radius);
        }
        let mut b: Vec<_> = (0..w.bush_slots.len())
            .map(|s| {
                (
                    crate::world::wrapped_dist2(w.bush_slots[s], [45.0, 120.0]),
                    s,
                )
            })
            .collect();
        b.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (d, s) in b.iter().take(3) {
            println!("bush {s} at {:?} dist {}", w.bush(*s, 0).pos, d.sqrt());
        }
    }
}
