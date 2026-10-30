//! The world every client and the server derive from one seed: a heightfield that wraps at the
//! edges, lakes carved into it, and where trees, boulders and berry bushes stand.

use crate::noise::{fbm, hash3, hash64, unit};

/// Every build shares this seed, so all clients and the server stand in the same world.
pub const WORLD_SEED: u64 = 1;
/// Side of the square world in metres. Positions wrap modulo this.
pub const WORLD_SIZE: f32 = 512.0;
/// Heightfield spacing in metres. The terrain mesh uses exactly these samples, two triangles per
/// cell split along the (0,0)-(1,1) diagonal, and [`World::height`] interpolates the same way.
pub const CELL: f32 = 2.0;
/// Samples per side.
pub const GRID: usize = (WORLD_SIZE / CELL) as usize;
/// Height of every lake's surface.
pub const WATER_LEVEL: f32 = 0.0;
/// Ground lower than this above the water counts as water for walking: the shoreline wall.
pub const SHORE_MARGIN: f32 = 0.08;
/// How deep a deer can wade before the water stops it (metres below the surface).
pub const WADE_DEPTH: f32 = 0.7;
/// Berries a bush can hold at most (they are tracked as a `u16` mask).
pub const MAX_BERRIES: u32 = 16;

const LAKE_COUNT: u32 = 9;
// Big trees with wide crowns: few trunks, but the crowns close overhead.
const TREE_CELL: f32 = 7.0;
const BOULDER_CELL: f32 = 16.0;
const BUSH_CELL: f32 = 32.0;
const BUSHES_PER_CELL: u32 = 2;

/// Wraps a coordinate into `[0, WORLD_SIZE)`.
pub fn wrap(v: f32) -> f32 {
    v.rem_euclid(WORLD_SIZE)
}

/// Wraps a difference of coordinates into `[-WORLD_SIZE / 2, WORLD_SIZE / 2)`: the shortest way
/// around the world.
pub fn wrap_delta(d: f32) -> f32 {
    (d + WORLD_SIZE * 0.5).rem_euclid(WORLD_SIZE) - WORLD_SIZE * 0.5
}

/// Squared shortest distance between two points on the wrapped plane.
pub fn wrapped_dist2(a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = wrap_delta(a[0] - b[0]);
    let dz = wrap_delta(a[1] - b[1]);
    dx * dx + dz * dz
}

#[derive(Clone, Debug)]
pub struct Lake {
    pub center: [f32; 2],
    pub radius: f32,
    pub depth: f32,
}

#[derive(Clone, Debug)]
pub struct Tree {
    /// Index in `World::trees`.
    pub id: u32,
    pub pos: [f32; 2],
    /// Ground height under the trunk.
    pub y: f32,
    /// Random bits; the client picks a model from them.
    pub kind: u64,
    pub scale: f32,
    pub yaw: f32,
    /// Rough trunk radius, for keeping other things clear of it (the client measures the
    /// trunk of the model it draws for collisions).
    pub radius: f32,
}

#[derive(Clone, Debug)]
pub struct Boulder {
    pub pos: [f32; 2],
    pub y: f32,
    pub kind: u64,
    pub scale: f32,
    pub yaw: f32,
    pub radius: f32,
}

/// Where a bush spawns, for one generation of one slot.
#[derive(Clone, Debug)]
pub struct BushSpawn {
    pub pos: [f32; 2],
    pub y: f32,
    pub kind: u64,
    pub yaw: f32,
    pub scale: f32,
    pub berries: u32,
}

pub struct World {
    pub seed: u64,
    heights: Vec<f32>,
    pub lakes: Vec<Lake>,
    pub trees: Vec<Tree>,
    pub boulders: Vec<Boulder>,
    /// Base position of each bush slot; generations move a little around it.
    pub bush_slots: Vec<[f32; 2]>,
    /// Trees (indices into `trees`) per collision bucket, `BUCKETS` x `BUCKETS`.
    tree_buckets: Vec<Vec<u32>>,
}

const BUCKET: f32 = 8.0;
const BUCKETS: usize = (WORLD_SIZE / BUCKET) as usize;

impl World {
    pub fn generate(seed: u64) -> World {
        let lakes = gen_lakes(seed);
        let mut heights = vec![0.0; GRID * GRID];
        for j in 0..GRID {
            for i in 0..GRID {
                heights[j * GRID + i] = raw_height(seed, &lakes, i as f32 * CELL, j as f32 * CELL);
            }
        }
        let mut world = World {
            seed,
            heights,
            lakes,
            trees: Vec::new(),
            boulders: Vec::new(),
            bush_slots: Vec::new(),
            tree_buckets: vec![Vec::new(); BUCKETS * BUCKETS],
        };
        world.place_trees();
        world.place_boulders();
        world.place_bush_slots();
        world
    }

    /// Height sample at a lattice point (wrapping).
    pub fn sample(&self, i: i64, j: i64) -> f32 {
        let i = i.rem_euclid(GRID as i64) as usize;
        let j = j.rem_euclid(GRID as i64) as usize;
        self.heights[j * GRID + i]
    }

    /// Ground height, interpolated across the same triangles the terrain mesh draws.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let gx = x / CELL;
        let gz = z / CELL;
        let i = gx.floor();
        let j = gz.floor();
        let fx = gx - i;
        let fz = gz - j;
        let (i, j) = (i as i64, j as i64);
        let h00 = self.sample(i, j);
        let h10 = self.sample(i + 1, j);
        let h01 = self.sample(i, j + 1);
        let h11 = self.sample(i + 1, j + 1);
        if fx > fz {
            h00 + (h10 - h00) * fx + (h11 - h10) * fz
        } else {
            h00 + (h11 - h01) * fx + (h01 - h00) * fz
        }
    }

    /// Unnormalised surface normal from central differences (smooth, for placing things).
    pub fn normal(&self, x: f32, z: f32) -> [f32; 3] {
        let e = CELL * 0.5;
        let dx = self.height(x + e, z) - self.height(x - e, z);
        let dz = self.height(x, z + e) - self.height(x, z - e);
        let n = [-dx, 2.0 * e, -dz];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        [n[0] / l, n[1] / l, n[2] / l]
    }

    /// Whether this spot is lake (too wet to walk on).
    pub fn is_water(&self, x: f32, z: f32) -> bool {
        self.height(x, z) < WATER_LEVEL + SHORE_MARGIN
    }

    /// Whether the water here is too deep to wade into.
    pub fn too_deep(&self, x: f32, z: f32) -> bool {
        self.height(x, z) < WATER_LEVEL - WADE_DEPTH
    }

    /// Distance to the nearest lake's edge-ish, cheap: negative inside a lake's radius.
    pub fn lake_distance(&self, x: f32, z: f32) -> f32 {
        self.lakes
            .iter()
            .map(|l| wrapped_dist2([x, z], l.center).sqrt() - l.radius)
            .fold(f32::INFINITY, f32::min)
    }

    /// Calls `f` for every tree whose trunk may lie within `r` of `(x, z)`.
    pub fn trees_near(&self, x: f32, z: f32, r: f32, mut f: impl FnMut(&Tree)) {
        let reach = ((r + 1.0) / BUCKET).ceil() as i64;
        let bi = (wrap(x) / BUCKET) as i64;
        let bj = (wrap(z) / BUCKET) as i64;
        for dj in -reach..=reach {
            for di in -reach..=reach {
                let i = (bi + di).rem_euclid(BUCKETS as i64) as usize;
                let j = (bj + dj).rem_euclid(BUCKETS as i64) as usize;
                for &t in &self.tree_buckets[j * BUCKETS + i] {
                    f(&self.trees[t as usize]);
                }
            }
        }
    }

    /// Forest density in [0, 1]: low values are clearings.
    pub fn forest_density(&self, x: f32, z: f32) -> f32 {
        let n = fbm(hash64(self.seed ^ 0xF0E5), x, z, WORLD_SIZE, 128.0, 3, 0.5);
        // Mostly deep forest; meadows only where the noise dips low.
        (n * 1.6 + 0.85).clamp(0.0, 1.0)
    }

    /// Whether a tree would stand in one of the small sunlit gaps between the crowns.
    pub fn canopy_gap(&self, x: f32, z: f32) -> bool {
        fbm(hash64(self.seed ^ 0x6A95), x, z, WORLD_SIZE, 32.0, 3, 0.5) > 0.2
    }

    fn place_trees(&mut self) {
        let seed = hash64(self.seed ^ 0x7EE5);
        let n = (WORLD_SIZE / TREE_CELL) as u64;
        for j in 0..n {
            for i in 0..n {
                let h = hash3(seed, i, j);
                let x = (i as f32 + 0.15 + 0.7 * unit(hash64(h ^ 1))) * TREE_CELL;
                let z = (j as f32 + 0.15 + 0.7 * unit(hash64(h ^ 2))) * TREE_CELL;
                let density = self.forest_density(x, z);
                if unit(h) > density * 0.85 {
                    continue;
                }
                // Small holes in the canopy, where sunlight reaches the floor.
                if self.canopy_gap(x, z) {
                    continue;
                }
                // Keep trees off the water and a step back from the shore.
                if self.lake_distance(x, z) < 4.0 || self.height(x, z) < WATER_LEVEL + 0.6 {
                    continue;
                }
                let scale = 0.75 + 0.6 * unit(hash64(h ^ 3));
                let tree = Tree {
                    id: self.trees.len() as u32,
                    pos: [x, z],
                    y: self.min_height_around(x, z, 0.5),
                    kind: hash64(h ^ 4),
                    scale,
                    yaw: unit(hash64(h ^ 5)) * std::f32::consts::TAU,
                    radius: 0.9 * scale,
                };
                let b = ((x / BUCKET) as usize).min(BUCKETS - 1)
                    + ((z / BUCKET) as usize).min(BUCKETS - 1) * BUCKETS;
                self.tree_buckets[b].push(self.trees.len() as u32);
                self.trees.push(tree);
            }
        }
    }

    fn place_boulders(&mut self) {
        let seed = hash64(self.seed ^ 0xB01D);
        let n = (WORLD_SIZE / BOULDER_CELL) as u64;
        for j in 0..n {
            for i in 0..n {
                let h = hash3(seed, i, j);
                if unit(h) > 0.22 {
                    continue;
                }
                let x = (i as f32 + unit(hash64(h ^ 1))) * BOULDER_CELL;
                let z = (j as f32 + unit(hash64(h ^ 2))) * BOULDER_CELL;
                if self.is_water(x, z) || self.tree_within(x, z, 2.0) {
                    continue;
                }
                let scale = 0.6 + 1.0 * unit(hash64(h ^ 3));
                self.boulders.push(Boulder {
                    pos: [x, z],
                    y: self.min_height_around(x, z, 0.8 * scale),
                    kind: hash64(h ^ 4),
                    scale,
                    yaw: unit(hash64(h ^ 5)) * std::f32::consts::TAU,
                    radius: 0.8 * scale,
                });
            }
        }
    }

    fn place_bush_slots(&mut self) {
        let seed = hash64(self.seed ^ 0xB005);
        let n = (WORLD_SIZE / BUSH_CELL) as u64;
        for j in 0..n {
            for i in 0..n {
                for k in 0..BUSHES_PER_CELL as u64 {
                    // A few tries to land somewhere dry and clear of trunks.
                    for attempt in 0..8u64 {
                        let h = hash3(seed, i * 7 + k, j * 13 + attempt * 101);
                        let x = (i as f32 + unit(hash64(h ^ 1))) * BUSH_CELL;
                        let z = (j as f32 + unit(hash64(h ^ 2))) * BUSH_CELL;
                        if self.bush_spot_ok(x, z) {
                            self.bush_slots.push([x, z]);
                            break;
                        }
                    }
                }
            }
        }
    }

    fn bush_spot_ok(&self, x: f32, z: f32) -> bool {
        self.height(x, z) > WATER_LEVEL + 0.4
            && !self.tree_within(x, z, 1.2)
            && !self
                .boulders
                .iter()
                .any(|b| wrapped_dist2(b.pos, [x, z]) < (b.radius + 1.5).powi(2))
    }

    fn tree_within(&self, x: f32, z: f32, r: f32) -> bool {
        let mut hit = false;
        self.trees_near(x, z, r, |t| {
            if wrapped_dist2(t.pos, [x, z]) < (r + t.radius).powi(2) {
                hit = true;
            }
        });
        hit
    }

    fn min_height_around(&self, x: f32, z: f32, r: f32) -> f32 {
        [(0.0, 0.0), (r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r)]
            .iter()
            .map(|(dx, dz)| self.height(x + dx, z + dz))
            .fold(f32::INFINITY, f32::min)
    }

    /// The bush that grows in `slot` for its `generation`th time.
    pub fn bush(&self, slot: usize, generation: u32) -> BushSpawn {
        let base = self.bush_slots[slot];
        let mut pos = base;
        let mut h = hash3(self.seed ^ 0xBE77, slot as u64, generation as u64);
        // Each regrowth lands a little to the side of the last, still clear of trunks and water.
        if generation > 0 {
            for attempt in 0..6u64 {
                let hh = hash64(h ^ attempt);
                let x = wrap(base[0] + (unit(hh) - 0.5) * 5.0);
                let z = wrap(base[1] + (unit(hash64(hh)) - 0.5) * 5.0);
                if self.bush_spot_ok(x, z) {
                    pos = [x, z];
                    h = hh;
                    break;
                }
            }
        }
        BushSpawn {
            pos,
            y: self.min_height_around(pos[0], pos[1], 0.4),
            kind: hash64(h ^ 3),
            yaw: unit(hash64(h ^ 4)) * std::f32::consts::TAU,
            scale: 0.85 + 0.35 * unit(hash64(h ^ 5)),
            berries: 6 + (hash64(h ^ 6) % 5) as u32,
        }
    }

    /// A dry place to put a new deer, picked by `n`.
    pub fn spawn_point(&self, n: u64) -> [f32; 2] {
        for attempt in 0..64u64 {
            let h = hash3(self.seed ^ 0x5BA7, n, attempt);
            let x = unit(h) * WORLD_SIZE;
            let z = unit(hash64(h)) * WORLD_SIZE;
            if self.height(x, z) > WATER_LEVEL + 1.0 && !self.tree_within(x, z, 1.0) {
                return [x, z];
            }
        }
        [0.0, 0.0]
    }
}

fn gen_lakes(seed: u64) -> Vec<Lake> {
    let seed = hash64(seed ^ 0x1A4E);
    let mut lakes: Vec<Lake> = Vec::new();
    let mut attempt = 0u64;
    while lakes.len() < LAKE_COUNT as usize && attempt < 200 {
        let h = hash3(seed, attempt, 0);
        attempt += 1;
        let center = [unit(h) * WORLD_SIZE, unit(hash64(h)) * WORLD_SIZE];
        // Mostly ponds, the odd small lake.
        let r = unit(hash64(h ^ 7));
        let radius = 7.0 + 26.0 * r * r;
        if lakes
            .iter()
            .any(|l| wrapped_dist2(l.center, center).sqrt() < l.radius + radius + 40.0)
        {
            continue;
        }
        lakes.push(Lake {
            center,
            radius,
            depth: 1.2 + radius * 0.05,
        });
    }
    lakes
}

/// Rolling hills, lifted so that only the carved lakes go below the water.
fn base_height(seed: u64, x: f32, z: f32) -> f32 {
    let hills = fbm(seed, x, z, WORLD_SIZE, 128.0, 3, 0.45);
    let detail = fbm(hash64(seed ^ 0xD7), x, z, WORLD_SIZE, 16.0, 3, 0.5);
    // Ridged ups and downs: sharper crests than plain noise.
    let ridge = 1.0 - fbm(hash64(seed ^ 0x41), x, z, WORLD_SIZE, 64.0, 2, 0.5).abs() * 2.0;
    let h = 3.0 + hills * 9.0 + ridge * 1.6 + detail * 0.6;
    // Valleys flatten into meadows that level out half a metre above the water.
    if h < 1.5 {
        1.5 - (1.5 - h) / (1.0 + (1.5 - h))
    } else {
        h
    }
}

fn raw_height(seed: u64, lakes: &[Lake], x: f32, z: f32) -> f32 {
    let mut h = base_height(seed, x, z);
    for lake in lakes {
        let dx = wrap_delta(x - lake.center[0]);
        let dz = wrap_delta(z - lake.center[1]);
        // A wobbly shoreline: the radius varies with direction (via noise on the position).
        let wobble = 1.0 + 0.25 * fbm(hash64(seed ^ 0x5E), x, z, WORLD_SIZE, 16.0, 2, 0.5);
        let d = (dx * dx + dz * dz).sqrt() / (lake.radius * wobble);
        // Bowl below the water inside the lake, then a gentle shore rising away from it.
        let profile = if d < 1.0 {
            WATER_LEVEL - lake.depth * (1.0 - d * d)
        } else {
            WATER_LEVEL + (d - 1.0) * lake.radius * 0.18
        };
        if profile < h {
            // Blend the shore into the hills instead of a hard crease.
            let t = ((d - 1.0) / 1.5).clamp(0.0, 1.0);
            let blend = t * t * (3.0 - 2.0 * t);
            h = profile + (h - profile) * blend;
        }
    }
    h
}
