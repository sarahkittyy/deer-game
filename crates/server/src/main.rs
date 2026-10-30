//! The deer game server: one shared world. Clients send their deer's pose; the server relays
//! everyone's poses back as snapshots, and owns the berry bushes so a berry eaten by one deer is
//! gone for all of them.

mod wolf;

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use deer_shared::{
    protocol::{self, BushState, ClientMsg, EAT_REACH, PROTOCOL_VERSION, Pose, ServerMsg, TICK_HZ},
    world::{World, wrapped_dist2},
};
use futures::{SinkExt, StreamExt};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// A bush with no berries left is replaced by a new one after this long.
const REGROW: Duration = Duration::from_secs(25);
const MAX_PLAYERS: usize = 256;
/// Messages a client may send per second before the extras are dropped.
const MAX_MSGS_PER_SEC: u32 = 40;
/// Shortest time between two chat lines from one deer; faster ones are dropped.
const CHAT_GAP: Duration = Duration::from_millis(900);
/// Frames queued for a slow client before it is cut off.
const SEND_QUEUE: usize = 256;

struct Player {
    name: String,
    stag: bool,
    pose: Option<Pose>,
    tx: mpsc::Sender<Vec<u8>>,
}

struct Shared {
    world: World,
    bushes: Vec<BushState>,
    regrow_at: Vec<Option<Instant>>,
    players: HashMap<u32, Player>,
    next_id: u32,
    wolves: wolf::Pack,
}

type AppState = Arc<Mutex<Shared>>;

fn full_mask(n: u32) -> u16 {
    ((1u32 << n) - 1) as u16
}

impl Shared {
    fn broadcast(&mut self, msg: &ServerMsg, except: Option<u32>) {
        let bytes = protocol::encode(msg);
        for (&id, p) in &self.players {
            if Some(id) != except {
                // A full queue means the client stopped reading; its socket task notices and ends.
                let _ = p.tx.try_send(bytes.clone());
            }
        }
    }

    fn tick(&mut self, now: Instant) {
        for slot in 0..self.bushes.len() {
            if self.regrow_at[slot].is_some_and(|t| now >= t) {
                self.regrow_at[slot] = None;
                let generation = self.bushes[slot].generation + 1;
                let spawn = self.world.bush(slot, generation);
                let state = BushState {
                    generation,
                    berries: full_mask(spawn.berries),
                };
                self.bushes[slot] = state;
                self.broadcast(
                    &ServerMsg::Bush {
                        slot: slot as u16,
                        state,
                    },
                    None,
                );
            }
        }
        let poses: Vec<(u32, Pose)> = self
            .players
            .iter()
            .filter_map(|(&id, p)| p.pose.map(|pose| (id, pose)))
            .collect();
        // Two steps a tick: wolves turn and dodge trunks at speed.
        let mut events = Vec::new();
        for _ in 0..2 {
            self.wolves
                .tick(0.5 / TICK_HZ, &self.world, &poses, &mut events);
        }
        for ev in &events {
            self.broadcast(ev, None);
        }
        if !poses.is_empty() {
            self.broadcast(&ServerMsg::Snapshot { poses }, None);
            let wolves = self.wolves.states(&self.world);
            self.broadcast(&ServerMsg::Wolves(wolves), None);
        }
    }

    fn eat(&mut self, id: u32, slot: u16, generation: u32, berry: u8) {
        let Some(pose) = self.players.get(&id).and_then(|p| p.pose) else {
            return;
        };
        let s = slot as usize;
        let Some(state) = self.bushes.get_mut(s) else {
            return;
        };
        let bit = 1u16.checked_shl(berry as u32).unwrap_or(0);
        if state.generation != generation || state.berries & bit == 0 {
            return;
        }
        let bush = self.world.bush(s, generation);
        if wrapped_dist2(bush.pos, [pose.pos[0], pose.pos[2]]) > EAT_REACH * EAT_REACH {
            return;
        }
        state.berries &= !bit;
        let state = *state;
        if state.berries == 0 {
            self.regrow_at[s] = Some(Instant::now() + REGROW);
        }
        self.broadcast(&ServerMsg::Ate { id, slot, berry }, None);
        self.broadcast(&ServerMsg::Bush { slot, state }, None);
    }
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // A fresh forest every time the server starts, unless pinned with DEER_SEED.
    let seed: u64 = env_or("DEER_SEED", rand::random());
    let host: String = env_or("DEER_HOST", "0.0.0.0".to_string());
    let port: u16 = env_or("DEER_PORT", 3560);

    let world = World::generate(seed);
    let bushes: Vec<BushState> = (0..world.bush_slots.len())
        .map(|slot| BushState {
            generation: 0,
            berries: full_mask(world.bush(slot, 0).berries),
        })
        .collect();
    info!(
        "world seed {seed}: {} trees, {} bushes, {} lakes",
        world.trees.len(),
        bushes.len(),
        world.lakes.len()
    );
    // Rare: a few wolves in the whole forest, unless DEER_WOLVES says otherwise.
    let wolves = wolf::Pack::new(&world, env_or("DEER_WOLVES", 5));
    let state: AppState = Arc::new(Mutex::new(Shared {
        wolves,
        world,
        regrow_at: vec![None; bushes.len()],
        bushes,
        players: HashMap::new(),
        next_id: 1,
    }));

    let ticker = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs_f32(1.0 / TICK_HZ));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            ticker.lock().unwrap().tick(Instant::now());
        }
    });

    let mut app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(|| async { "ok" }))
        .route("/version", get(|| async { PROTOCOL_VERSION.to_string() }))
        .with_state(state);
    // Local web testing: serve the web build too (production uses nginx for this).
    if let Ok(dir) = std::env::var("DEER_STATIC") {
        info!("serving {dir} at /");
        app = app.fallback_service(tower_http::services::ServeDir::new(dir).precompressed_gzip());
    }

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    info!("listening on {addr}");
    axum::serve(listener, app).await.expect("serve");
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.max_message_size(4096)
        .on_upgrade(move |socket| client(socket, state))
}

async fn client(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();

    // The first message must be the hello, and soon.
    let hello = tokio::time::timeout(Duration::from_secs(10), stream.next()).await;
    let Ok(Some(Ok(Message::Binary(bytes)))) = hello else {
        return;
    };
    let Some(ClientMsg::Hello {
        version,
        name,
        stag,
    }) = protocol::decode::<ClientMsg>(&bytes)
    else {
        return;
    };
    if version != PROTOCOL_VERSION {
        let msg = protocol::encode(&ServerMsg::WrongVersion {
            server: PROTOCOL_VERSION,
        });
        let _ = sink.send(Message::Binary(msg.into())).await;
        return;
    }
    let name = protocol::clean_name(&name);

    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(SEND_QUEUE);
    let id = {
        let mut s = state.lock().unwrap();
        if s.players.len() >= MAX_PLAYERS {
            return;
        }
        let id = s.next_id;
        s.next_id += 1;
        let spawn = s.world.spawn_point(id as u64 ^ rand::random::<u64>());
        let welcome = ServerMsg::Welcome {
            id,
            seed: s.world.seed,
            spawn,
            bushes: s.bushes.clone(),
        };
        let _ = tx.try_send(protocol::encode(&welcome));
        for (&other, p) in &s.players {
            let joined = ServerMsg::Joined {
                id: other,
                name: p.name.clone(),
                stag: p.stag,
            };
            let _ = tx.try_send(protocol::encode(&joined));
        }
        s.broadcast(
            &ServerMsg::Joined {
                id,
                name: name.clone(),
                stag,
            },
            None,
        );
        s.players.insert(
            id,
            Player {
                name: name.clone(),
                stag,
                pose: None,
                tx,
            },
        );
        id
    };
    info!("{id} joined as {name:?}");

    let mut send_task = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if sink.send(Message::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
    });

    let recv_state = state.clone();
    let mut recv_task = tokio::spawn(async move {
        let mut window = Instant::now();
        let mut count = 0u32;
        let mut last_chat: Option<Instant> = None;
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Binary(bytes) = msg else {
                if matches!(msg, Message::Close(_)) {
                    break;
                }
                continue;
            };
            if window.elapsed() >= Duration::from_secs(1) {
                window = Instant::now();
                count = 0;
            }
            count += 1;
            if count > MAX_MSGS_PER_SEC {
                continue;
            }
            match protocol::decode::<ClientMsg>(&bytes) {
                Some(ClientMsg::Pose(pose)) if pose_ok(&pose) => {
                    if let Some(p) = recv_state.lock().unwrap().players.get_mut(&id) {
                        p.pose = Some(pose);
                    }
                }
                Some(ClientMsg::Eat {
                    slot,
                    generation,
                    berry,
                }) => recv_state.lock().unwrap().eat(id, slot, generation, berry),
                Some(ClientMsg::Chat(raw)) => {
                    if last_chat.is_some_and(|t| t.elapsed() < CHAT_GAP) {
                        continue;
                    }
                    if let Some(text) = protocol::clean_chat(&raw) {
                        last_chat = Some(Instant::now());
                        info!("{id} says {text:?}");
                        // The sender shows its own line already.
                        recv_state
                            .lock()
                            .unwrap()
                            .broadcast(&ServerMsg::Chat { id, text }, Some(id));
                    }
                }
                Some(_) => {}
                None => warn!("{id}: undecodable message"),
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }

    let mut s = state.lock().unwrap();
    s.players.remove(&id);
    s.broadcast(&ServerMsg::Left { id }, None);
    info!("{id} left");
}

fn pose_ok(p: &Pose) -> bool {
    p.pos.iter().all(|v| v.is_finite() && v.abs() < 10_000.0)
        && [
            p.body_yaw,
            p.head_yaw,
            p.head_pitch,
            p.speed,
            p.pace,
            p.look,
        ]
        .iter()
        .chain(
            p.tumble
                .iter()
                .flat_map(|t| t.rot.iter().chain(&t.vel).chain([&t.fall])),
        )
        .all(|v| v.is_finite())
}
