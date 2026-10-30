//! Messages between the game and the server, bincode-encoded in binary WebSocket frames.

use serde::{Deserialize, Serialize};

/// Bumped whenever a message changes shape; the server refuses other versions.
pub const PROTOCOL_VERSION: u32 = 3;
/// How often each side sends poses / snapshots.
pub const TICK_HZ: f32 = 15.0;
pub const MAX_NAME_LEN: usize = 16;
/// Longest chat message, in characters.
pub const MAX_CHAT_LEN: usize = 80;
/// How close (metres) a deer must be to a bush for the server to accept eating from it.
pub const EAT_REACH: f32 = 4.5;

/// What the deer is doing besides moving, so ghosts can play the matching animation.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Action {
    #[default]
    None,
    Airborne,
    Eating,
    Drinking,
    /// Knocked flying by a wolf: tumbling, lying where it landed, or getting back up.
    Knocked,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    /// Wrapped into the world square.
    pub pos: [f32; 3],
    pub body_yaw: f32,
    pub head_yaw: f32,
    pub head_pitch: f32,
    /// Horizontal speed in m/s, to pick and pace walk/run.
    pub speed: f32,
    /// How fast this deer walks and gallops compared with usual (its heart: 0.5 to 1.5), so
    /// ghosts blend walk and gallop at the right speeds.
    pub pace: f32,
    pub action: Action,
    /// Where the player is looking (the camera's yaw), so wolves know when they are watched.
    pub look: f32,
    /// The whole body's turn while knocked flying.
    pub tumble: Option<Tumble>,
}

/// A knocked deer's body, for others to draw: no bones, just how it is turned and moving.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Tumble {
    /// Rotation of the whole body (a quaternion, x y z w) about its middle.
    pub rot: [f32; 4],
    pub vel: [f32; 3],
    /// How far through the collapse the legs are (0 standing, 1 lying flat): the death clip's
    /// time, played forwards to fall and backwards to get up.
    pub fall: f32,
}

/// Wolves: rare, hostile, and run by the server.
pub mod wolf {
    /// Deer within this many metres can be spotted.
    pub const SPOT_RADIUS: f32 = 31.0;
    pub const WANDER_SPEED: f32 = 1.5;
    /// The slow walk towards a deer that has stared too long (or looked away).
    pub const STALK_SPEED: f32 = 1.1;
    /// Chasing: this much faster than the deer's own top gallop.
    pub const CHASE_FACTOR: f32 = 1.12;
    /// Starts the pounce this far from the deer (about a deer's length, plus the leap's reach).
    pub const POUNCE_RANGE: f32 = 2.2;
    /// The pounce: a leap this long, hitting the deer this far into it.
    pub const LEAP_SECS: f32 = 0.55;
    pub const HIT_AT: f32 = 0.22;
}

/// What a wolf is up to.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WolfMood {
    #[default]
    Wander,
    /// Standing its ground, staring at a deer that is watching it, barking now and then.
    Stare,
    /// Walking slowly up to the deer.
    Stalk,
    Chase,
    Pounce,
    /// Done with the deer; stands a moment, then wanders off.
    Rest,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct WolfState {
    /// Wrapped into the world square.
    pub pos: [f32; 3],
    /// Horizontal velocity, m/s: clients carry the wolf on between snapshots with it.
    pub vel: [f32; 2],
    pub yaw: f32,
    pub mood: WolfMood,
    /// The deer it is after.
    pub target: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum WolfCall {
    Growl,
    Bark,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BushState {
    /// Times this slot has regrown; picks where and which bush grows (`World::bush`).
    pub generation: u32,
    /// Bit `i` set while berry `i` is still on the bush.
    pub berries: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ClientMsg {
    /// `stag`: antlers (true) or a doe (false).
    Hello {
        version: u32,
        name: String,
        stag: bool,
    },
    Pose(Pose),
    Eat {
        slot: u16,
        generation: u32,
        berry: u8,
    },
    /// A line of chat, shown over the sender's head.
    Chat(String),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ServerMsg {
    Welcome {
        id: u32,
        seed: u64,
        spawn: [f32; 2],
        bushes: Vec<BushState>,
    },
    /// The server runs a different protocol; the page should reload.
    WrongVersion {
        server: u32,
    },
    Joined {
        id: u32,
        name: String,
        stag: bool,
    },
    Left {
        id: u32,
    },
    Snapshot {
        poses: Vec<(u32, Pose)>,
    },
    Bush {
        slot: u16,
        state: BushState,
    },
    /// Someone ate a berry; lets clients play the sound and remove it at once.
    Ate {
        id: u32,
        slot: u16,
        berry: u8,
    },
    /// Every wolf, by index, each tick.
    Wolves(Vec<WolfState>),
    WolfCall {
        wolf: u8,
        call: WolfCall,
    },
    /// A wolf leaps at a deer from `from` along `dir`; the deer's own game knocks it flying
    /// `wolf::HIT_AT` seconds later.
    Pounce {
        wolf: u8,
        victim: u32,
        from: [f32; 3],
        dir: [f32; 2],
    },
    /// Someone said something (already cleaned by `clean_chat`).
    Chat {
        id: u32,
        text: String,
    },
}

fn config() -> impl bincode::config::Config {
    bincode::config::standard().with_limit::<{ 1 << 20 }>()
}

pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    bincode::serde::encode_to_vec(msg, config()).expect("encoding a message cannot fail")
}

pub fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Option<T> {
    bincode::serde::decode_from_slice(bytes, config())
        .ok()
        .map(|(m, _)| m)
}

/// Trims a chat line to something printable and short; `None` if nothing is left.
pub fn clean_chat(raw: &str) -> Option<String> {
    let text: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_CHAT_LEN)
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Trims a requested name to something printable and short. Empty becomes "deer".
pub fn clean_name(raw: &str) -> String {
    let name: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_LEN)
        .collect();
    let name = name.trim().to_string();
    if name.is_empty() { "deer".into() } else { name }
}
