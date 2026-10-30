//! Messages between the game and the server, bincode-encoded in binary WebSocket frames.

use serde::{Deserialize, Serialize};

/// Bumped whenever a message changes shape; the server refuses other versions.
pub const PROTOCOL_VERSION: u32 = 1;
/// How often each side sends poses / snapshots.
pub const TICK_HZ: f32 = 15.0;
pub const MAX_NAME_LEN: usize = 16;
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
    pub action: Action,
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
