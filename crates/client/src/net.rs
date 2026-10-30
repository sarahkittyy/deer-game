//! The connection to the server: a WebSocket carrying bincode messages. Without a server the
//! game still plays, alone, and keeps trying to connect in the background.

use crate::{AppState, deer::DeerAnim, player::LocalDeer};
use bevy::prelude::*;
use deer_shared::protocol::{
    self, BushState, ClientMsg, PROTOCOL_VERSION, Pose, ServerMsg, TICK_HZ,
};
use ewebsock::{WsEvent, WsMessage, WsReceiver, WsSender};
use std::sync::Mutex;

const RETRY_SECS: f32 = 5.0;

/// Who the player is, from the title screen.
#[derive(Resource, Clone)]
pub struct Profile {
    pub name: String,
    pub stag: bool,
}

struct Socket(WsSender, WsReceiver);

// SAFETY: the browser build is single-threaded (wasm32 without atomics), so the socket's
// `Rc` handles never cross a thread.
#[cfg(target_arch = "wasm32")]
unsafe impl Send for Socket {}

#[derive(Resource, Default)]
pub struct Net {
    // Behind a mutex only to make the resource `Sync`; it is only used with `&mut Net`.
    socket: Option<Mutex<Socket>>,
    open: bool,
    pub my_id: Option<u32>,
    since_send: f32,
    since_attempt: f32,
}

/// Something the server told us, for other modules to react to.
#[derive(Message, Clone, Debug)]
pub enum NetEvent {
    /// The server's world has a different seed: `GameWorld` has just been regenerated, and
    /// everything built from the old one must be rebuilt.
    NewWorld,
    Welcome {
        spawn: [f32; 2],
        bushes: Vec<BushState>,
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
    Ate {
        id: u32,
        slot: u16,
        berry: u8,
    },
    Disconnected,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Net>()
        .add_message::<NetEvent>()
        .add_systems(PreUpdate, (keep_connected, receive).chain())
        .add_systems(Update, send_pose.run_if(in_state(AppState::Playing)));
}

pub fn server_url() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let loc = web_sys::window().unwrap().location();
        let proto = if loc.protocol().unwrap_or_default() == "https:" {
            "wss"
        } else {
            "ws"
        };
        let host = loc.host().unwrap_or_default();
        format!("{proto}://{host}/ws")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var("DEER_SERVER")
            .ok()
            .or(option_env!("DEER_SERVER").map(String::from))
            .unwrap_or_else(|| "ws://127.0.0.1:3560/ws".into())
    }
}

impl Net {
    pub fn connect(&mut self) {
        self.since_attempt = 0.0;
        self.open = false;
        self.my_id = None;
        match ewebsock::connect(server_url(), ewebsock::Options::default()) {
            Ok((tx, rx)) => self.socket = Some(Mutex::new(Socket(tx, rx))),
            Err(e) => {
                warn!("cannot connect: {e}");
                self.socket = None;
            }
        }
    }

    pub fn connected(&self) -> bool {
        self.my_id.is_some()
    }

    pub fn send(&mut self, msg: &ClientMsg) {
        if self.open
            && let Some(socket) = &mut self.socket
        {
            socket
                .get_mut()
                .unwrap()
                .0
                .send(WsMessage::Binary(protocol::encode(msg)));
        }
    }
}

fn keep_connected(time: Res<Time>, profile: Option<Res<Profile>>, mut net: ResMut<Net>) {
    // Only once the player has pressed play; then retry while the socket is gone.
    if profile.is_none() {
        return;
    }
    net.since_attempt += time.delta_secs();
    if net.socket.is_none() && net.since_attempt > RETRY_SECS {
        net.connect();
    }
}

fn receive(
    mut net: ResMut<Net>,
    mut world: ResMut<crate::GameWorld>,
    profile: Option<Res<Profile>>,
    mut events: MessageWriter<NetEvent>,
) {
    let mut lost = false;
    loop {
        let Some(socket) = &mut net.socket else { break };
        let Some(ev) = socket.get_mut().unwrap().1.try_recv() else {
            break;
        };
        match ev {
            WsEvent::Opened => {
                net.open = true;
                if let Some(p) = &profile {
                    net.send(&ClientMsg::Hello {
                        version: PROTOCOL_VERSION,
                        name: p.name.clone(),
                        stag: p.stag,
                    });
                }
            }
            WsEvent::Message(WsMessage::Binary(bytes)) => {
                let Some(msg) = protocol::decode::<ServerMsg>(&bytes) else {
                    continue;
                };
                let ev = match msg {
                    ServerMsg::Welcome {
                        id,
                        seed,
                        spawn,
                        bushes,
                    } => {
                        if seed != world.0.seed {
                            info!("server's world has seed {seed}; generating it");
                            world.0 =
                                std::sync::Arc::new(deer_shared::world::World::generate(seed));
                            events.write(NetEvent::NewWorld);
                        }
                        net.my_id = Some(id);
                        NetEvent::Welcome { spawn, bushes }
                    }
                    ServerMsg::WrongVersion { server } => {
                        warn!("server speaks protocol {server}, we speak {PROTOCOL_VERSION}");
                        #[cfg(target_arch = "wasm32")]
                        let _ = web_sys::window().unwrap().location().reload();
                        lost = true;
                        break;
                    }
                    ServerMsg::Joined { id, name, stag } => NetEvent::Joined { id, name, stag },
                    ServerMsg::Left { id } => NetEvent::Left { id },
                    ServerMsg::Snapshot { poses } => NetEvent::Snapshot { poses },
                    ServerMsg::Bush { slot, state } => NetEvent::Bush { slot, state },
                    ServerMsg::Ate { id, slot, berry } => NetEvent::Ate { id, slot, berry },
                };
                events.write(ev);
            }
            WsEvent::Message(_) => {}
            WsEvent::Error(e) => {
                warn!("connection error: {e}");
                lost = true;
                break;
            }
            WsEvent::Closed => {
                lost = true;
                break;
            }
        }
    }
    if lost {
        net.socket = None;
        net.open = false;
        net.my_id = None;
        net.since_attempt = 0.0;
        events.write(NetEvent::Disconnected);
    }
}

fn send_pose(time: Res<Time>, mut net: ResMut<Net>, deer: Query<(&LocalDeer, &DeerAnim)>) {
    net.since_send += time.delta_secs();
    if net.since_send < 1.0 / TICK_HZ || !net.connected() {
        return;
    }
    net.since_send = 0.0;
    if let Ok((d, anim)) = deer.single() {
        net.send(&ClientMsg::Pose(d.pose(anim)));
    }
}
