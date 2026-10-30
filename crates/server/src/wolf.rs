//! Wolves: a few hostile animals the server runs for everyone. A wolf wanders until a deer comes
//! within sight, then stares it down, barking. Watched for too long (or looked away from), it
//! walks slowly up to the deer; a deer that turns and runs is chased at a little over its own
//! top speed. Within about a deer's length the wolf pounces, and the deer's own game knocks it
//! flying. Then the wolf loses interest for a while.

use deer_shared::{
    protocol::{
        Action, Pose, ServerMsg, WolfCall, WolfMood, WolfState,
        wolf::{CHASE_FACTOR, LEAP_SECS, POUNCE_RANGE, SPOT_RADIUS, STALK_SPEED, WANDER_SPEED},
    },
    world::{WATER_LEVEL, World, wrap, wrap_delta},
};
use std::collections::HashSet;
use std::f32::consts::{PI, TAU};
use tracing::info;

/// The deer's walk and top gallop at an ordinary pace (as in the client's `deer.rs`).
const DEER_WALK: f32 = 1.9;
const DEER_RUN: f32 = 7.5;
/// How long a deer can stare a wolf down before it starts walking up anyway.
const STARE_LIMIT: f32 = 10.0;
/// A deer further than this gets away.
const LOSE_RADIUS: f32 = 48.0;
/// A chase this long ends in the wolf giving up.
const CHASE_GIVE_UP: f32 = 30.0;
/// After an attack (or losing a deer), how long before the wolf picks on anyone again.
const AGGRO_COOLDOWN: f32 = 10.0;
const LOST_COOLDOWN: f32 = 4.0;
const REST_SECS: f32 = 2.0;
const BODY_RADIUS: f32 = 0.4;
/// The chance that a new wander leg heads to somewhere near a deer, so wolves come across
/// players now and then in a big forest.
const SEEK_DEER: f32 = 0.3;
/// After the bark that says it has seen a deer, the wolf gives it this long to stop and look
/// round before running counts as fleeing: a deer galloping past is not charged at once.
const NOTICE_SECS: f32 = 1.0;
/// Looking within this angle of a wolf counts as watching it.
const WATCH_COS: f32 = 0.57;

#[derive(Clone, Copy, Debug, Default)]
struct V(f32, f32);

impl V {
    fn len(self) -> f32 {
        (self.0 * self.0 + self.1 * self.1).sqrt()
    }
    fn norm(self) -> V {
        let l = self.len().max(1e-5);
        V(self.0 / l, self.1 / l)
    }
    fn dot(self, o: V) -> f32 {
        self.0 * o.0 + self.1 * o.1
    }
    fn scale(self, k: f32) -> V {
        V(self.0 * k, self.1 * k)
    }
    fn add(self, o: V) -> V {
        V(self.0 + o.0, self.1 + o.1)
    }
}

/// The shortest way from `a` to `b` on the wrapped plane.
fn towards(a: V, b: V) -> V {
    V(wrap_delta(b.0 - a.0), wrap_delta(b.1 - a.1))
}

/// Unit vector for a yaw; yaw 0 faces -Z, like the deer.
fn facing(yaw: f32) -> V {
    V(-yaw.sin(), -yaw.cos())
}

fn yaw_of(d: V) -> f32 {
    (-d.0).atan2(-d.1)
}

fn angle_diff(a: f32, b: f32) -> f32 {
    (a - b + PI).rem_euclid(TAU) - PI
}

fn chance(p: f32) -> bool {
    rand::random::<f32>() < p
}

struct Wolf {
    pos: V,
    yaw: f32,
    vel: V,
    mood: WolfMood,
    target: Option<u32>,
    /// Seconds in the current mood.
    t: f32,
    /// How long the target has watched this wolf, all told.
    watched: f32,
    /// Seconds to the next bark or growl.
    call_in: f32,
    goal: V,
    /// Standing still a moment while wandering.
    pause: f32,
    /// Picks on no deer until this runs out.
    calm: f32,
    leap: V,
    /// Counts down the `NOTICE_SECS` after spotting a deer.
    notice: f32,
}

pub struct Pack {
    wolves: Vec<Wolf>,
    /// `DEER_WOLF_TEST=metres`: deer that have had a wolf put that far in front of them
    /// (behind, if negative).
    test_met: Option<HashSet<u32>>,
    test_dist: f32,
}

impl Pack {
    pub fn new(world: &World, count: usize) -> Pack {
        let wolves = (0..count)
            .map(|_| {
                let p = world.spawn_point(rand::random());
                let pos = V(p[0], p[1]);
                Wolf {
                    pos,
                    yaw: rand::random::<f32>() * TAU,
                    vel: V::default(),
                    mood: WolfMood::Wander,
                    target: None,
                    t: 0.0,
                    watched: 0.0,
                    call_in: 0.0,
                    goal: pos,
                    pause: 0.0,
                    calm: 0.0,
                    leap: V::default(),
                    notice: 0.0,
                }
            })
            .collect();
        Pack {
            wolves,
            test_met: std::env::var("DEER_WOLF_TEST").is_ok().then(HashSet::new),
            test_dist: std::env::var("DEER_WOLF_TEST")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(18.0),
        }
    }

    pub fn states(&self, world: &World) -> Vec<WolfState> {
        self.wolves
            .iter()
            .map(|w| WolfState {
                pos: [w.pos.0, world.height(w.pos.0, w.pos.1), w.pos.1],
                vel: [w.vel.0, w.vel.1],
                yaw: w.yaw,
                mood: w.mood,
                target: w.target,
            })
            .collect()
    }

    /// Moves every wolf on by `dt` seconds; what they do that clients must hear about goes into
    /// `out`.
    pub fn tick(&mut self, dt: f32, world: &World, deer: &[(u32, Pose)], out: &mut Vec<ServerMsg>) {
        if let Some(met) = &mut self.test_met {
            // Testing: a wolf appears in front of each deer as it arrives.
            for (id, pose) in deer {
                // One wolf each, for as many deer as there are wolves.
                if met.len() >= self.wolves.len() || !met.insert(*id) {
                    continue;
                }
                let n = (met.len() - 1) % self.wolves.len();
                let w = &mut self.wolves[n];
                let p = V(pose.pos[0], pose.pos[2]).add(facing(pose.look).scale(self.test_dist));
                w.pos = V(wrap(p.0), wrap(p.1));
                w.yaw = yaw_of(towards(w.pos, V(pose.pos[0], pose.pos[2])));
                w.mood = WolfMood::Wander;
                w.target = None;
                w.calm = 0.0;
                w.pause = 3.0;
            }
        }
        for (i, w) in self.wolves.iter_mut().enumerate() {
            w.tick(i as u8, dt, world, deer, out);
        }
    }
}

impl Wolf {
    fn set_mood(&mut self, mood: WolfMood) {
        self.mood = mood;
        self.t = 0.0;
    }

    fn give_up(&mut self, cooldown: f32) {
        self.set_mood(WolfMood::Wander);
        self.target = None;
        self.calm = cooldown;
        self.pause = 1.5;
        self.watched = 0.0;
    }

    fn tick(
        &mut self,
        id: u8,
        dt: f32,
        world: &World,
        deer: &[(u32, Pose)],
        out: &mut Vec<ServerMsg>,
    ) {
        self.t += dt;
        self.calm -= dt;
        self.call_in -= dt;
        self.notice -= dt;
        let mut call = |c: WolfCall| out.push(ServerMsg::WolfCall { wolf: id, call: c });
        let mut want = V::default();
        let mut face: Option<f32> = None;

        match self.mood {
            WolfMood::Wander | WolfMood::Rest => {
                if self.mood == WolfMood::Rest && self.t > REST_SECS {
                    self.set_mood(WolfMood::Wander);
                    self.pause = 0.0;
                    self.goal = self.pick_goal(world, deer);
                }
                if self.mood == WolfMood::Wander {
                    if self.pause > 0.0 {
                        self.pause -= dt;
                    } else {
                        let to = towards(self.pos, self.goal);
                        if to.len() < 2.0 {
                            self.pause = 2.0 + rand::random::<f32>() * 6.0;
                            self.goal = self.pick_goal(world, deer);
                        } else {
                            want = to.norm().scale(WANDER_SPEED);
                        }
                    }
                    // The first deer to come within sight.
                    if self.calm <= 0.0 {
                        let seen = deer
                            .iter()
                            .filter(|(_, p)| p.action != Action::Knocked)
                            .map(|(id, p)| (*id, towards(self.pos, V(p.pos[0], p.pos[2])).len()))
                            .filter(|(_, d)| *d < SPOT_RADIUS)
                            .min_by(|a, b| a.1.total_cmp(&b.1));
                        if let Some((target, _)) = seen {
                            info!("wolf {id} spots {target}");
                            self.target = Some(target);
                            self.watched = 0.0;
                            self.notice = NOTICE_SECS;
                            self.set_mood(WolfMood::Stare);
                            call(WolfCall::Bark);
                            self.call_in = 1.5 + rand::random::<f32>() * 2.0;
                        }
                    }
                }
            }
            WolfMood::Stare | WolfMood::Stalk | WolfMood::Chase => {
                let Some(pose) = self
                    .target
                    .and_then(|t| deer.iter().find(|(id, _)| *id == t))
                    .map(|(_, p)| *p)
                else {
                    self.give_up(LOST_COOLDOWN);
                    return self.finish(dt, world, want, None);
                };
                let at = V(pose.pos[0], pose.pos[2]);
                let to = towards(self.pos, at);
                let dist = to.len();
                if pose.action == Action::Knocked
                    || dist > LOSE_RADIUS
                    || (self.mood == WolfMood::Chase && self.t > CHASE_GIVE_UP)
                {
                    self.give_up(LOST_COOLDOWN);
                    return self.finish(dt, world, want, None);
                }
                if dist < POUNCE_RANGE {
                    self.leap = to.norm();
                    info!("wolf {id} pounces on {:?}", self.target);
                    self.set_mood(WolfMood::Pounce);
                    out.push(ServerMsg::Pounce {
                        wolf: id,
                        victim: self.target.unwrap_or_default(),
                        from: [self.pos.0, world.height(self.pos.0, self.pos.1), self.pos.1],
                        dir: [self.leap.0, self.leap.1],
                    });
                    return self.finish(dt, world, want, Some(yaw_of(self.leap)));
                }
                let pace = if pose.pace > 0.0 { pose.pace } else { 1.0 };
                let heading = facing(pose.body_yaw);
                let fleeing = self.notice <= 0.0
                    && pose.speed > DEER_WALK * pace * 1.4
                    && heading.dot(to.norm()) > 0.2;
                let watching = facing(pose.look).dot(to.norm().scale(-1.0)) > WATCH_COS;
                if watching {
                    self.watched += dt;
                }
                if fleeing && self.mood != WolfMood::Chase {
                    info!("wolf {id} chases {:?}", self.target);
                    self.set_mood(WolfMood::Chase);
                    call(WolfCall::Bark);
                }
                match self.mood {
                    WolfMood::Stare => {
                        face = Some(yaw_of(to));
                        // Not creeping up during the moment it gives the deer to notice it.
                        if self.notice <= 0.0 && (!watching || self.watched > STARE_LIMIT) {
                            info!("wolf {id} stalks (watched {:.1} s)", self.watched);
                            self.set_mood(WolfMood::Stalk);
                        } else if self.call_in <= 0.0 {
                            call(if chance(0.7) {
                                WolfCall::Bark
                            } else {
                                WolfCall::Growl
                            });
                            self.call_in = 2.5 + rand::random::<f32>() * 2.5;
                        }
                    }
                    WolfMood::Stalk => {
                        want = to.norm().scale(STALK_SPEED);
                        if watching && self.watched < STARE_LIMIT {
                            self.set_mood(WolfMood::Stare);
                        } else if self.call_in <= 0.0 {
                            call(WolfCall::Growl);
                            self.call_in = 3.0 + rand::random::<f32>() * 3.0;
                        }
                    }
                    _ => {
                        // Head for where the deer will be, a moment from now.
                        let speed = DEER_RUN * pace.max(0.5) * CHASE_FACTOR;
                        let lead = (dist / speed).min(0.5);
                        let ahead = to.add(heading.scale(pose.speed * lead));
                        want = ahead.norm().scale(speed);
                    }
                }
            }
            WolfMood::Pounce => {
                if self.t > LEAP_SECS {
                    self.set_mood(WolfMood::Rest);
                    self.target = None;
                    self.calm = AGGRO_COOLDOWN + REST_SECS;
                    self.watched = 0.0;
                } else {
                    // A leap of a few metres, carrying through where the deer stood.
                    want = self.leap.scale(POUNCE_RANGE * 2.0 / LEAP_SECS);
                    face = Some(yaw_of(self.leap));
                }
            }
        }
        self.finish(dt, world, want, face);
    }

    /// Eases the velocity towards `want`, moves with collisions and turns the body.
    fn finish(&mut self, dt: f32, world: &World, want: V, face: Option<f32>) {
        let rate = match self.mood {
            WolfMood::Chase | WolfMood::Pounce => 10.0,
            _ => 5.0,
        };
        let k = 1.0 - (-dt * rate).exp();
        self.vel = self
            .vel
            .add(V(want.0 - self.vel.0, want.1 - self.vel.1).scale(k));

        // One axis at a time, so it slides along the water's edge.
        let mut blocked = false;
        for axis in [V(1.0, 0.0), V(0.0, 1.0)] {
            let step = V(self.vel.0 * axis.0, self.vel.1 * axis.1).scale(dt);
            let next = self.pos.add(step);
            if world.height(next.0, next.1) < WATER_LEVEL - 0.35 {
                blocked = true;
                if axis.0 > 0.0 {
                    self.vel.0 = 0.0;
                } else {
                    self.vel.1 = 0.0;
                }
            } else {
                self.pos = next;
            }
        }
        if blocked && self.mood == WolfMood::Wander {
            self.goal = V(
                self.pos.0 + (rand::random::<f32>() - 0.5) * 80.0,
                self.pos.1 + (rand::random::<f32>() - 0.5) * 80.0,
            );
        }
        self.pos = push_out(world, self.pos);
        self.pos = V(wrap(self.pos.0), wrap(self.pos.1));

        let speed = self.vel.len();
        let want_yaw = face.or((speed > 0.3).then(|| yaw_of(self.vel)));
        if let Some(y) = want_yaw {
            let max = if speed > 4.0 { 7.0 } else { 4.0 } * dt;
            self.yaw = (self.yaw + angle_diff(y, self.yaw).clamp(-max, max)).rem_euclid(TAU);
        }
    }

    /// Somewhere dry to wander to: usually nearby, sometimes over towards a deer.
    fn pick_goal(&self, world: &World, deer: &[(u32, Pose)]) -> V {
        for _ in 0..8 {
            let a = rand::random::<f32>() * TAU;
            let g = if !deer.is_empty() && chance(SEEK_DEER) {
                let p = deer[rand::random_range(0..deer.len())].1.pos;
                V(p[0], p[2]).add(facing(a).scale(30.0 + rand::random::<f32>() * 30.0))
            } else {
                self.pos
                    .add(facing(a).scale(25.0 + rand::random::<f32>() * 45.0))
            };
            let g = V(wrap(g.0), wrap(g.1));
            if world.height(g.0, g.1) > WATER_LEVEL + 0.5 {
                return g;
            }
        }
        self.pos
    }
}

/// Pushes the wolf's body circle out of trunks and boulders.
fn push_out(world: &World, p: V) -> V {
    let mut push = V::default();
    let mut resolve = |c: [f32; 2], r: f32| {
        let d = towards(V(c[0], c[1]), p);
        let min = r + BODY_RADIUS;
        let len = d.len();
        if len < min {
            push = push.add(d.norm().scale(min - len));
        }
    };
    world.trees_near(p.0, p.1, 4.0, |t| resolve(t.pos, t.radius));
    for b in &world.boulders {
        if (wrap_delta(b.pos[0] - p.0)).abs() < 6.0 && (wrap_delta(b.pos[1] - p.1)).abs() < 6.0 {
            resolve(b.pos, b.radius);
        }
    }
    p.add(push)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deer galloping past is given a second to stop before the wolf gives chase.
    #[test]
    fn notice_before_chase() {
        let world = World::generate(1);
        let mut pack = Pack::new(&world, 1);
        pack.test_met = None;
        let at = world.spawn_point(0);
        let w = &mut pack.wolves[0];
        w.pos = V(at[0], at[1]);
        w.calm = 0.0;
        // 10 m off, galloping straight away and looking where it goes.
        let away = -std::f32::consts::FRAC_PI_2;
        let pose = Pose {
            pos: [wrap(at[0] + 10.0), 0.0, at[1]],
            body_yaw: away,
            look: away,
            speed: DEER_RUN,
            pace: 1.0,
            ..Default::default()
        };
        let deer = [(7, pose)];
        let mut out = Vec::new();
        let mut moods = Vec::new();
        for _ in 0..30 {
            pack.tick(0.05, &world, &deer, &mut out);
            moods.push(pack.wolves[0].mood);
        }
        assert_eq!(pack.wolves[0].target, Some(7));
        // Seen (and barked at) straight away, but no chase within the first second.
        assert!(matches!(
            out.first(),
            Some(ServerMsg::WolfCall {
                call: WolfCall::Bark,
                ..
            })
        ));
        assert!(
            moods[..19].iter().all(|m| *m == WolfMood::Stare),
            "{moods:?}"
        );
        assert_eq!(moods[29], WolfMood::Chase);
    }
}
