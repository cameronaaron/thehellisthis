#[cfg(test)]
mod tests;

use axum::extract::ws::{Message, WebSocket};
use axum::{
    extract::{ConnectInfo, FromRequestParts, Path, State, WebSocketUpgrade},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Json, Router,
};
use axum_server::Server;
use bytes::Bytes;
use comrak::{markdown_to_html, Options as ComrakOptions};
use futures::{SinkExt, StreamExt};
use http::request::Parts;
use http::{header, HeaderMap, StatusCode};
use lazy_static::lazy_static;
use rand::prelude::SliceRandom;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::{broadcast, Mutex, RwLock};
use tower_cookies::Cookie;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

// Constants and settings
const MAX_ROOMS: usize = 100;
const MAX_ROOM_NAME_LEN: usize = 50;
const MIN_ROOM_NAME_LEN: usize = 3;
const ROOM_NAME_REGEX: &str = "^[a-zA-Z0-9][a-zA-Z0-9-_]*[a-zA-Z0-9]$";

const MAX_MESSAGE_LEN: usize = 8000;
const INACTIVE_TIMEOUT: Duration = Duration::from_secs(3600);
const MESSAGE_RATE_LIMIT: Duration = Duration::from_millis(500);
const MAX_MESSAGES_PER_WINDOW: usize = 30;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(6);
const ROOM_CLEANUP_INTERVAL: Duration = Duration::from_secs(60); // Check every 1 minute
const EMPTY_ROOM_CLEANUP_DELAY: Duration = Duration::from_secs(60); // Delete empty rooms after 1 min
const MAX_TOTAL_ROOMS_MEMORY: usize = 400_000_000;
const MAX_MESSAGES_PER_ROOM: usize = 500;
const MAX_MESSAGE_AGE: Duration = Duration::from_secs(86400 * 30);
const CLEANUP_BATCH_SIZE: usize = 100;
const TYPING_EVENT_MIN_INTERVAL: Duration = Duration::from_millis(200);
const READ_RECEIPT_MIN_INTERVAL: Duration = Duration::from_millis(200);

// Memory + reconnection
const ESTIMATED_MESSAGE_SIZE: usize = 1024;

// Rate limiting
const MAX_CONCURRENT_CONNECTIONS_PER_IP: usize = 3;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);
const MAX_ROOM_JOIN_ATTEMPTS: usize = 10;
const SANITIZE_TIMEOUT: Duration = Duration::from_millis(50);
const MAX_PAYLOAD_SIZE: usize = 512 * 1024;

// Per-user constraints
const MAX_CONCURRENT_USERS: usize = 400;

// Health check
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
struct RateLimiter {
    window_start: Instant,
    message_count: usize,
    join_attempts: usize,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            window_start: Instant::now(),
            message_count: 0,
            join_attempts: 0,
        }
    }

    fn can_send_message(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_start) > RATE_LIMIT_WINDOW {
            self.window_start = now;
            self.message_count = 0;
        }

        if self.message_count >= MAX_MESSAGES_PER_WINDOW {
            return false;
        }

        self.message_count += 1;
        true
    }

    fn can_join_room(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_start) > RATE_LIMIT_WINDOW {
            self.window_start = now;
            self.join_attempts = 0;
        }
        self.join_attempts += 1;
        self.join_attempts <= MAX_ROOM_JOIN_ATTEMPTS
    }
}

#[derive(Debug)]
struct ResourceMonitor {
    total_memory: AtomicUsize,
    total_connections: AtomicUsize,
}

impl ResourceMonitor {
    fn new() -> Self {
        Self {
            total_memory: AtomicUsize::new(0),
            total_connections: AtomicUsize::new(0),
        }
    }

    fn can_accept_connection(&self) -> bool {
        let conn_count = self.total_connections.load(Ordering::Relaxed);
        let mem_usage = self.total_memory.load(Ordering::Relaxed);
        conn_count < MAX_CONCURRENT_USERS && mem_usage < MAX_TOTAL_ROOMS_MEMORY
    }
}

/// A single struct that tracks both user_id and animal_name from cookies.
/// We'll use this to keep the same user across reloads.
#[derive(Debug, Clone)]
struct UserCookie {
    user_id: String,
    animal_name: String,
}

/// Wrapper for optional UserCookie
#[derive(Debug, Clone)]
struct OptionalUserCookie(Option<UserCookie>);

/// Parse both `user_id` and `animal_name` cookies from the request (if present).
impl<S> FromRequestParts<S> for OptionalUserCookie
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        info!("Attempting to extract UserCookie from request headers...");

        let mut user_id = None;
        let mut animal_name = None;

        if let Some(cookie_str) = parts.headers.get("cookie").and_then(|v| v.to_str().ok()) {
            let cookies: Vec<Cookie> = cookie_str
                .split(';')
                .filter_map(|s| Cookie::parse(s.trim().to_string()).ok())
                .collect();

            for c in &cookies {
                match c.name() {
                    "user_id" => {
                        user_id = Some(c.value().to_string());
                    }
                    "animal_name" => {
                        animal_name = Some(c.value().to_string());
                    }
                    _ => {}
                }
            }
        }

        if let (Some(u), Some(a)) = (user_id, animal_name) {
            if !u.is_empty() && !a.is_empty() {
                info!("Found valid user_id [{}] and animal_name [{}]", u, a);
                return Ok(OptionalUserCookie(Some(UserCookie {
                    user_id: u,
                    animal_name: a,
                })));
            }
        }

        info!("No valid user_id and animal_name cookies found, treating as new user.");
        Ok(OptionalUserCookie(None))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct OutgoingMessage {
    message_id: Uuid,
    user_id: String, // ** Added user_id to differentiate server-side
    animal_name: String,
    text: String,
    timestamp: String,
}

// Add message size estimation method
impl OutgoingMessage {
    fn estimate_size(&self) -> usize {
        self.user_id.len()
            + self.animal_name.len()
            + self.text.len()
            + self.timestamp.len()
            + std::mem::size_of::<Uuid>()
            + ESTIMATED_MESSAGE_SIZE
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
enum SystemEvent {
    UserJoined {
        user_id: String,
        animal_name: String,
    },
    UserLeft {
        user_id: String,
        animal_name: String,
    },
    Typing {
        user_id: String,
        animal_name: String,
        is_typing: bool,
    },
    ReadReceipt {
        user_id: String,
        animal_name: String,
        message_id: Uuid,
    },
    ServerShutdown {
        reason: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
enum OutgoingEvent {
    Message { message: OutgoingMessage },
    System { event: SystemEvent },
    UserCount { count: usize },
    Heartbeat,
    ReconnectToken { token: String },
}

// Connection states
#[derive(Debug, Clone, PartialEq)]
enum ConnectionState {
    Connected {
        last_heartbeat: Instant,
        connection_id: String,
    },
    Disconnected {
        since: Instant,
    },
}

#[derive(Clone)]
struct UserData {
    user_id: String,
    animal_name: String,
    last_active: Instant,
    last_message_time: Instant,
    connection_state: ConnectionState,
    last_read_message: Option<Uuid>,
    is_typing: bool,
    last_typing_event: Option<Instant>,
    last_read_receipt_event: Option<Instant>,
    rate_limiter: RateLimiter,
    last_sanitized_message: Option<(String, Instant)>,
}

struct RoomState {
    last_activity: Instant,
    total_memory_bytes: AtomicUsize,
    users: HashMap<String, UserData>,
    available_animals: VecDeque<String>,
    sender: broadcast::Sender<OutgoingEvent>,
    chat_history: Vec<OutgoingMessage>,
}

#[derive(Error, Debug)]
pub enum ChatError {
    #[error("Room is full")]
    RoomFull,
    #[error("Rate limit exceeded")]
    RateLimited,
    #[error("Invalid message: {0}")]
    InvalidMessage(String),
    #[error("Resource limit exceeded: {0}")]
    ResourceLimit(String),
    #[error("Connection error: {0}")]
    ConnectionError(String),
    #[error("Security error: {0}")]
    SecurityError(String),
    #[error("Room operation failed: {0}")]
    RoomError(String),
    #[error("Rate limit exceeded: {0}")]
    RateLimitError(String),
}

impl IntoResponse for ChatError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ChatError::RoomFull => (StatusCode::SERVICE_UNAVAILABLE, "Room is full"),
            ChatError::RateLimited | ChatError::RateLimitError(_) => {
                (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded")
            }
            ChatError::InvalidMessage(_) => (StatusCode::BAD_REQUEST, "Invalid message"),
            ChatError::ResourceLimit(_) => (StatusCode::SERVICE_UNAVAILABLE, "Server at capacity"),
            ChatError::ConnectionError(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Connection error")
            }
            ChatError::SecurityError(_) => (StatusCode::FORBIDDEN, "Access denied"),
            ChatError::RoomError(_) => (StatusCode::BAD_REQUEST, "Room operation failed"),
        };

        let body = serde_json::json!({
            "error": message,
            "details": self.to_string()
        });

        (status, Json(body)).into_response()
    }
}

type ChatResult<T> = Result<T, ChatError>;

// Memory tracking
struct MemoryTracker {
    total_bytes: AtomicUsize,
    peak_bytes: AtomicUsize,
    last_gc: AtomicU64,
}

impl MemoryTracker {
    fn new() -> Self {
        Self {
            total_bytes: AtomicUsize::new(0),
            peak_bytes: AtomicUsize::new(0),
            last_gc: AtomicU64::new(0),
        }
    }

    fn add_bytes(&self, bytes: usize) -> bool {
        let total = self.total_bytes.fetch_add(bytes, Ordering::SeqCst) + bytes;
        if total > MAX_TOTAL_ROOMS_MEMORY {
            self.total_bytes.fetch_sub(bytes, Ordering::SeqCst);
            return false;
        }

        let mut peak = self.peak_bytes.load(Ordering::Relaxed);
        while total > peak {
            match self.peak_bytes.compare_exchange_weak(
                peak,
                total,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(p) => peak = p,
            }
        }
        true
    }

    fn remove_bytes(&self, bytes: usize) {
        // Use saturating_sub to prevent underflow
        let current = self.total_bytes.load(Ordering::SeqCst);
        let new_val = current.saturating_sub(bytes);
        self.total_bytes.store(new_val, Ordering::SeqCst);
    }

    fn should_gc(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let last = self.last_gc.load(Ordering::Relaxed);
        if now - last > 300 {
            // Update last_gc timestamp
            self.last_gc.store(now, Ordering::SeqCst);
            true
        } else {
            false
        }
    }
}

// Connection pooling
struct ConnectionPool {
    active: AtomicUsize,
    ip_counters: RwLock<HashMap<String, (AtomicUsize, Instant)>>,
}

impl ConnectionPool {
    fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            ip_counters: RwLock::new(HashMap::new()),
        }
    }

    async fn cleanup_stale(&self) {
        let mut counters = self.ip_counters.write().await;
        counters.retain(|_, (_, last_seen)| last_seen.elapsed() < Duration::from_secs(3600));
    }

    async fn can_accept(&self, ip: &str) -> bool {
        let counters = self.ip_counters.read().await;
        if let Some((counter, _)) = counters.get(ip) {
            counter.load(Ordering::Relaxed) < MAX_CONCURRENT_CONNECTIONS_PER_IP
        } else {
            true
        }
    }

    async fn add_connection(&self, ip: &str) -> ChatResult<()> {
        let mut counters = self.ip_counters.write().await;
        let (counter, last_seen) = counters
            .entry(ip.to_string())
            .or_insert_with(|| (AtomicUsize::new(0), Instant::now()));

        *last_seen = Instant::now();

        if counter.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_CONNECTIONS_PER_IP {
            counter.fetch_sub(1, Ordering::SeqCst);
            return Err(ChatError::ResourceLimit(
                "Too many connections from IP".into(),
            ));
        }

        if self.active.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_USERS {
            counter.fetch_sub(1, Ordering::SeqCst);
            self.active.fetch_sub(1, Ordering::SeqCst);
            return Err(ChatError::ResourceLimit("Server at capacity".into()));
        }

        Ok(())
    }

    async fn remove_connection(&self, ip: &str) {
        let mut counters = self.ip_counters.write().await;
        if let Some((counter, _)) = counters.get_mut(ip) {
            counter.fetch_sub(1, Ordering::SeqCst);
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

// Security
struct SecurityManager {
    banned_ips: RwLock<HashMap<String, Instant>>,
    suspicious_activity: RwLock<HashMap<String, (usize, Instant)>>,
}

impl SecurityManager {
    fn new() -> Self {
        Self {
            banned_ips: RwLock::new(HashMap::new()),
            suspicious_activity: RwLock::new(HashMap::new()),
        }
    }

    async fn check_ip(&self, ip: &str) -> ChatResult<()> {
        let banned = self.banned_ips.read().await;
        if let Some(ban_time) = banned.get(ip) {
            if ban_time.elapsed() < Duration::from_secs(3600) {
                return Err(ChatError::SecurityError("IP is banned".into()));
            }
        }
        Ok(())
    }

    async fn record_suspicious_activity(&self, ip: &str) -> ChatResult<()> {
        let mut suspicious = self.suspicious_activity.write().await;
        let (count, first_seen) = suspicious
            .entry(ip.to_string())
            .or_insert_with(|| (0, Instant::now()));

        *count += 1;

        if *count > 10 && first_seen.elapsed() < Duration::from_secs(60) {
            let mut banned = self.banned_ips.write().await;
            banned.insert(ip.to_string(), Instant::now());
            return Err(ChatError::SecurityError(
                "Too many suspicious activities".into(),
            ));
        }
        Ok(())
    }
}

struct AppState {
    rooms: RwLock<HashMap<String, RoomState>>,
    resource_monitor: ResourceMonitor,
    memory_tracker: MemoryTracker,
    connection_pool: ConnectionPool,
    security_manager: SecurityManager,
}

fn create_room() -> RoomState {
    info!("Creating a new room with a large, diverse set of animal names...");
    let mut animals = vec![
        "dog",
        "cat",
        "lion",
        "tiger",
        "elephant",
        "giraffe",
        "koala",
        "penguin",
        "panda",
        "dolphin",
        "whale",
        "bear",
        "wolf",
        "zebra",
        "fox",
        "owl",
        "rabbit",
        "kangaroo",
        "monkey",
        "snake",
        "parrot",
        "cheetah",
        "jaguar",
        "lynx",
        "otter",
        "seal",
        "peacock",
        "sparrow",
        "crow",
        "hedgehog",
        "flamingo",
        "shark",
        "stingray",
        "starfish",
        "octopus",
        "seahorse",
        "crab",
        "lobster",
        "squid",
        "antelope",
        "badger",
        "bison",
        "buffalo",
        "camel",
        "chameleon",
        "crocodile",
        "eagle",
        "ferret",
        "gecko",
        "gorilla",
        "heron",
        "hyena",
        "ibis",
        "iguana",
        "lemur",
        "leopard",
        "manatee",
        "mole",
        "moose",
        "narwhal",
        "newt",
        "ostrich",
        "platypus",
        "porcupine",
        "raven",
        "salamander",
        "sloth",
        "stork",
        "tapir",
        "toad",
        "turkey",
        "vulture",
        "wallaby",
        "walrus",
        "wolverine",
        "yak",
        "hippo",
        "rhino",
        "anteater",
        "armadillo",
        "beaver",
        "butterfly",
        "cormorant",
        "coyote",
        "dingo",
        "dragonfly",
        "firefly",
        "grasshopper",
        "hamster",
        "honeyeater",
        "hummingbird",
        "kingfisher",
        "ladybug",
        "llama",
        "meerkat",
        "moth",
        "ox",
        "puffin",
        "quail",
        "ringtail",
        "swan",
        "tortoise",
        "turtle",
        "woodpecker",
        "wombat",
        "orangutan",
        "manta-ray",
        "robin",
        "musk-ox",
        "kiwi",
        "harpy-eagle",
        "peafowl",
        "margay",
        "capybara",
        "urchin",
        "bandicoot",
        "guinea-pig",
        "axolotl",
        "dugong",
        "fennec-fox",
        "pika",
        "tamarin",
        "aardwolf",
        "colugo",
        "dhole",
        "galago",
        "alpaca",
        "anaconda",
        "antlion",
        "auk",
        "aye-aye",
        "basilisk",
        "bee",
        "beetle",
        "bengal-cat",
        "binturong",
        "bird-of-paradise",
        "bonobo",
        "booby",
        "bushbaby",
        "caracal",
        "caracara",
        "cassowary",
        "centipede",
        "chinchilla",
        "chipmunk",
        "civet",
        "clownfish",
        "coati",
        "cobra",
        "cockatiel",
        "cockatoo",
        "conure",
        "copperhead",
        "cuttlefish",
        "damselfly",
        "deer",
        "devil-ray",
        "dodo",
        "donkey",
        "dove",
        "dung-beetle",
        "emu",
        "ermine",
        "falcon",
        "fallow-deer",
        "fathead-minnow",
        "flapjack-octopus",
        "flatfish",
        "flightless-cormorant",
        "flounder",
        "flying-fish",
        "flying-lemur",
        "flying-squirrel",
        "frilled-lizard",
        "frog",
        "fruit-fly",
        "fulmar",
        "gayal",
        "gavial",
        "gazelle",
        "gibbon",
        "glass-lizard",
        "glowworm",
        "gnu",
        "goat",
        "goby",
        "godwit",
        "goldcrest",
        "goldfinch",
        "goldfish",
        "goosander",
        "goose",
        "gopher",
        "goral",
        "goshawk",
        "gosling",
        "grackle",
        "gray-whale",
        "grebe",
        "greyhound",
        "griffin",
        "grouse",
        "grouper",
        "guanaco",
        "guillemot",
        "guinea-fowl",
        "gull",
        "guppy",
        "gurami",
        "gurnard",
        "gymnure",
        "gypsy-moth",
        "gyri",
        "gyroscope",
        "habu",
        "haddock",
        "hadji",
        "hadron",
        "hagborn",
        "hagfish",
        "haggadic",
        "haggis",
        "haggler",
        "hagiology",
        "hagioscope",
        "haj",
        "hajj",
        "hajji",
        "hake",
        "hakim",
        "halal",
        "halbe",
        "halbert",
        "halcyon",
        "hale",
        "half-back",
        "half-beak",
        "half-blood",
        "half-cock",
        "half-crab",
        "half-cutter",
        "half-day",
        "half-deck",
        "half-penny",
        "half-track",
        "halibut",
        "halid",
        "halide",
        "halidome",
        "halif",
        "halimeda",
        "haliotis",
        "halite",
        "halitus",
        "halk",
        "hall",
        "hallabaloo",
        "hallal",
        "hallan",
        "hallel",
        "hallelujah",
        "haller",
        "halley",
        "halliday",
        "hallide",
        "hallier",
        "hallified",
        "halliford",
        "halliform",
        "halligan",
        "hallikainen",
        "hallikon",
        "halliland",
        "hallilot",
        "hallily",
        "hallimeda",
        "hallimond",
        "hallingers",
        "hallings",
        "hallingers",
        "hallion",
        "hallionic",
        "halliotidae",
        "halliotis",
        "hallish",
        "hallis",
        "hallissey",
        "hallistor",
        "halliwell",
        "hallawine",
        "halloween",
        "hallowmas",
        "hallows",
        "halloums",
        "halls",
        "hallstatt",
        "hallux",
        "hallway",
        "hallways",
        "hallway",
        "hallwort",
    ];
    animals.shuffle(&mut rand::thread_rng());
    let (tx, _) = broadcast::channel::<OutgoingEvent>(1000);

    RoomState {
        last_activity: Instant::now(),
        total_memory_bytes: AtomicUsize::new(0),
        users: HashMap::new(),
        available_animals: animals.into_iter().map(String::from).collect(),
        sender: tx,
        chat_history: Vec::new(),
    }
}

impl RoomState {
    fn assign_animal(&mut self) -> String {
        info!("Assigning an animal from the pool to a new user...");
        for _ in 0..self.available_animals.len() {
            if let Some(animal) = self.available_animals.pop_front() {
                if !self.users.values().any(|u| {
                    matches!(u.connection_state, ConnectionState::Connected { .. })
                        && u.animal_name == animal
                }) {
                    info!("Assigned animal: {}", animal);
                    return animal;
                }
                self.available_animals.push_back(animal);
            }
        }
        let guest_num = self.users.len() + 1;
        let name = format!("guest_{}", guest_num);
        info!("No free animal found, assigned guest name: {}", name);
        name
    }

    fn add_message(&mut self, msg: OutgoingMessage, memory_tracker: &MemoryTracker) {
        self.last_activity = Instant::now();
        let msg_size = msg.estimate_size();

        let current_memory = self.total_memory_bytes.load(Ordering::Relaxed);
        if current_memory + msg_size > MAX_TOTAL_ROOMS_MEMORY {
            self.prune_old_messages(msg_size, memory_tracker);
        }

        if !memory_tracker.add_bytes(msg_size) {
            warn!("Dropping message because memory budget exceeded");
            return;
        }

        self.total_memory_bytes
            .fetch_add(msg_size, Ordering::SeqCst);
        self.chat_history.push(msg);
    }

    fn prune_old_messages(&mut self, needed_space: usize, memory_tracker: &MemoryTracker) {
        let mut removed_size = 0;
        while removed_size < needed_space && !self.chat_history.is_empty() {
            if let Some(msg) = self.chat_history.first() {
                removed_size += msg.estimate_size();
                self.chat_history.remove(0);
            }
        }
        let current = self.total_memory_bytes.load(Ordering::SeqCst);
        self.total_memory_bytes
            .store(current.saturating_sub(removed_size), Ordering::SeqCst);

        if removed_size > 0 {
            memory_tracker.remove_bytes(removed_size);
        }
    }

    fn broadcast_user_count(&self) {
        let now = Instant::now();
        let connected_count = self
            .users
            .values()
            .filter(|u| match &u.connection_state {
                ConnectionState::Connected { last_heartbeat, .. } => {
                    now.duration_since(*last_heartbeat) <= HEARTBEAT_TIMEOUT
                }
                _ => false,
            })
            .count();

        info!("Broadcasting user count: {}", connected_count);
        let _ = self.sender.send(OutgoingEvent::UserCount {
            count: connected_count,
        });
    }

    fn broadcast_system_event(&self, event: SystemEvent) {
        info!("Broadcasting system event: {:?}", event);
        let _ = self.sender.send(OutgoingEvent::System { event });
    }

    async fn cleanup_messages(&mut self, _now: Instant, memory_tracker: &MemoryTracker) {
        let mut removed = 0;
        let mut removed_bytes = 0;
        let mut messages_to_retain = Vec::new();

        let current_time_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        for msg in self.chat_history.iter() {
            if removed >= CLEANUP_BATCH_SIZE {
                messages_to_retain.push(msg.clone());
                continue;
            }

            if let Ok(msg_time_ms) = msg.timestamp.parse::<u128>() {
                let age_ms = current_time_ms.saturating_sub(msg_time_ms);
                if age_ms <= MAX_MESSAGE_AGE.as_millis() {
                    messages_to_retain.push(msg.clone());
                } else {
                    removed += 1;
                    removed_bytes += msg.estimate_size();
                }
            } else {
                messages_to_retain.push(msg.clone());
            }
        }

        if removed > 0 {
            self.chat_history = messages_to_retain;
            let new_total = self
                .chat_history
                .iter()
                .map(|msg| msg.estimate_size())
                .sum::<usize>();
            self.total_memory_bytes.store(new_total, Ordering::SeqCst);
            // Update global memory tracker
            memory_tracker.remove_bytes(removed_bytes);
        }
    }

    fn preserve_messages(&mut self, memory_tracker: &MemoryTracker) {
        if self.chat_history.len() > MAX_MESSAGES_PER_ROOM + 100 {
            let start_idx = self.chat_history.len() - MAX_MESSAGES_PER_ROOM;
            // Compute bytes we are about to drop so the global tracker stays accurate
            let removed_bytes = self.chat_history[..start_idx]
                .iter()
                .map(|msg| msg.estimate_size())
                .sum::<usize>();

            self.chat_history = self.chat_history[start_idx..].to_vec();

            let new_total = self
                .chat_history
                .iter()
                .map(|msg| msg.estimate_size())
                .sum::<usize>();
            self.total_memory_bytes.store(new_total, Ordering::SeqCst);

            if removed_bytes > 0 {
                memory_tracker.remove_bytes(removed_bytes);
            }
        }
    }

    fn trim_to_max_messages(&mut self, memory_tracker: &MemoryTracker) {
        if self.chat_history.len() > MAX_MESSAGES_PER_ROOM {
            let start_idx = self.chat_history.len() - MAX_MESSAGES_PER_ROOM;
            let removed_bytes = self.chat_history[..start_idx]
                .iter()
                .map(|msg| msg.estimate_size())
                .sum::<usize>();

            self.chat_history = self.chat_history[start_idx..].to_vec();

            let new_total = self
                .chat_history
                .iter()
                .map(|msg| msg.estimate_size())
                .sum::<usize>();
            self.total_memory_bytes.store(new_total, Ordering::SeqCst);

            if removed_bytes > 0 {
                memory_tracker.remove_bytes(removed_bytes);
            }
        }
    }

    #[allow(dead_code)]
    fn is_user_allowed(&mut self, user_id: &str) -> bool {
        let connected_count = self
            .users
            .values()
            .filter(|u| matches!(u.connection_state, ConnectionState::Connected { .. }))
            .count();

        if connected_count >= 100 {
            return false;
        }
        if let Some(user) = self.users.get_mut(user_id) {
            user.rate_limiter.can_join_room()
        } else {
            true
        }
    }

    async fn trigger_cleanup(&mut self, memory_tracker: &MemoryTracker) {
        if self.total_memory_bytes.load(Ordering::Relaxed) > (MAX_TOTAL_ROOMS_MEMORY * 9) / 10 {
            self.cleanup_messages(Instant::now(), memory_tracker).await;
        }
    }
}

impl AppState {
    fn new() -> Self {
        info!("Initializing AppState...");
        Self {
            rooms: RwLock::new(HashMap::with_capacity(MAX_ROOMS)),
            resource_monitor: ResourceMonitor::new(),
            memory_tracker: MemoryTracker::new(),
            connection_pool: ConnectionPool::new(),
            security_manager: SecurityManager::new(),
        }
    }

    async fn shutdown(&self) {
        info!("Beginning graceful shutdown...");
        let mut rooms = self.rooms.write().await;
        for (_room_name, room_state) in rooms.iter_mut() {
            for (_user_id, user) in room_state.users.iter_mut() {
                if let ConnectionState::Connected { .. } = user.connection_state {
                    let _ = room_state.sender.send(OutgoingEvent::System {
                        event: SystemEvent::UserLeft {
                            user_id: user.user_id.clone(),
                            animal_name: user.animal_name.clone(),
                        },
                    });
                }
            }
        }
        rooms.clear();
        info!("Shutdown complete");
    }

    async fn cleanup(&self) {
        self.connection_pool.cleanup_stale().await;
        if self.memory_tracker.should_gc() {
            let mut rooms = self.rooms.write().await;
            let total_memory = rooms
                .values()
                .map(|r| r.total_memory_bytes.load(Ordering::Relaxed))
                .sum::<usize>();
            if total_memory > MAX_TOTAL_ROOMS_MEMORY {
                for room in rooms.values_mut() {
                    room.trigger_cleanup(&self.memory_tracker).await;
                }
            }
        }
    }
}

/// Creates two cookies for a user: `user_id` and `animal_name`.
fn create_user_cookies(user_id: &str, name: &str) -> (String, String) {
    info!(
        "Creating cookies for user_id [{}] with animal_name [{}]",
        user_id, name
    );

    let user_id_cookie = format!(
        "user_id={}; Path=/; Max-Age={}; SameSite=Strict; Secure",
        user_id,
        INACTIVE_TIMEOUT.as_secs()
    );
    let animal_name_cookie = format!(
        "animal_name={}; Path=/; Max-Age={}; SameSite=Strict; Secure",
        name,
        INACTIVE_TIMEOUT.as_secs()
    );
    (user_id_cookie, animal_name_cookie)
}

async fn root_redirect() -> Redirect {
    info!("Received request at '/', redirecting to '/main'");
    Redirect::permanent("/main")
}

/// Serves the main chat page at /main
async fn main_room_handler() -> impl IntoResponse {
    info!("HTTP request for main room");
    let html = include_str!("../index.html");
    Html(html.to_string())
}

/// Serves the dynamic room page
async fn room_handler(
    Path(room): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    info!("HTTP request for room: {}", room);

    let reserved_paths = [
        "robots.txt",
        "sitemap.xml",
        "favicon.ico",
        ".well-known",
        "main",
        "admin",
        "api",
        "health",
        "metrics",
        "ws",
    ];
    if reserved_paths.contains(&room.as_str()) {
        warn!("Attempted to access reserved path as room: {}", room);
        return Html("Invalid room name".to_string());
    }

    if room.len() < MIN_ROOM_NAME_LEN || room.len() > MAX_ROOM_NAME_LEN {
        warn!("Room name length invalid: {}", room);
        return Html("Room name must be between 3 and 50 characters".to_string());
    }

    lazy_static! {
        static ref ROOM_REGEX: Regex = Regex::new(ROOM_NAME_REGEX).unwrap();
    }
    if !ROOM_REGEX.is_match(&room) {
        warn!("Room name format invalid: {}", room);
        return Html("Room name must start/end with alphanumeric chars".to_string());
    }

    let rooms = state.rooms.read().await;
    if !rooms.contains_key(&room) && rooms.len() >= MAX_ROOMS {
        warn!(
            "Max rooms reached ({}) and requested room {} does not exist",
            MAX_ROOMS, room
        );
        return Html("Maximum number of rooms reached".to_string());
    }

    if room.len() > MAX_ROOM_NAME_LEN
        || !room
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        warn!("Invalid room name requested: {}", room);
        return Html("Invalid room name".to_string());
    }

    let html = include_str!("../index.html");
    Html(html.to_string())
}

/// Validate basic text input
fn validate_input(text: &str, max_len: usize) -> Result<(), &'static str> {
    if text.is_empty() {
        return Err("Input cannot be empty");
    }
    if text.len() > max_len {
        return Err("Input too long");
    }
    if !text
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Input contains invalid characters");
    }
    Ok(())
}

/// Extract client IP from headers or connection info
fn extract_client_ip(
    headers: &HeaderMap,
    conn_info: Option<&ConnectInfo<SocketAddr>>,
) -> Option<String> {
    // Try X-Forwarded-For first (for reverse proxies like Fly.io, nginx)
    if let Some(forwarded) = headers.get("x-forwarded-for") {
        if let Ok(forwarded_str) = forwarded.to_str() {
            // X-Forwarded-For can contain multiple IPs, take the first (client)
            if let Some(client_ip) = forwarded_str.split(',').next() {
                let ip = client_ip.trim().to_string();
                if !ip.is_empty() {
                    return Some(ip);
                }
            }
        }
    }

    // Try X-Real-IP (alternative header)
    if let Some(real_ip) = headers.get("x-real-ip") {
        if let Ok(ip_str) = real_ip.to_str() {
            let ip = ip_str.trim().to_string();
            if !ip.is_empty() {
                return Some(ip);
            }
        }
    }

    // Fall back to direct connection IP
    conn_info.map(|ci| ci.0.ip().to_string())
}

/// Upgrades the connection to WebSocket for the specified room
#[axum::debug_handler]
async fn ws_handler(
    State(state): State<Arc<AppState>>,
    conn_info: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    OptionalUserCookie(cookie): OptionalUserCookie,
    Path(room): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let ip = extract_client_ip(&headers, Some(&conn_info));

    match ws_handler_inner(state, ip, cookie, room, ws).await {
        Ok(response) => response.into_response(),
        Err(e) => e.into_response(),
    }
}

/// Inner implementation for ws_handler
async fn ws_handler_inner(
    state: Arc<AppState>,
    ip: Option<String>,
    cookie: Option<UserCookie>,
    room: String,
    ws: WebSocketUpgrade,
) -> Result<Response, ChatError> {
    info!(
        "WebSocket upgrade request for room: {} from IP: {:?}",
        room, ip
    );

    if let Some(ip) = &ip {
        state.security_manager.check_ip(ip).await?;
        if !state.connection_pool.can_accept(ip).await {
            let _ = state.security_manager.record_suspicious_activity(ip).await;
            return Err(ChatError::RateLimitError(
                "Too many connections from your IP".to_string(),
            ));
        }
        state.connection_pool.add_connection(ip).await?;
    }

    if !state.resource_monitor.can_accept_connection() {
        return Err(ChatError::ResourceLimit(
            "Server is at capacity".to_string(),
        ));
    }

    if let Err(e) = validate_input(&room, MAX_ROOM_NAME_LEN) {
        return Err(ChatError::InvalidMessage(e.to_string()));
    }

    state
        .resource_monitor
        .total_connections
        .fetch_add(1, Ordering::SeqCst);

    let connection_id = Uuid::new_v4().to_string();
    info!("New WebSocket connection ID: {}", connection_id);

    // Check room capacity
    {
        let rooms = state.rooms.read().await;
        if let Some(room_state) = rooms.get(&room) {
            let connected_count = room_state
                .users
                .values()
                .filter(|u| matches!(u.connection_state, ConnectionState::Connected { .. }))
                .count();
            if connected_count >= 100 {
                error!("Room {} connection limit reached", room);
                return Err(ChatError::RoomFull);
            }
        }
    }

    // Determine the user_id and animal_name. If there's a cookie, try to reuse.
    let (final_user_id, final_animal_name, user_id_cookie_str, animal_name_cookie_str) = {
        let mut rooms = state.rooms.write().await;
        let room_state = rooms.entry(room.clone()).or_insert_with(|| {
            info!("Creating room '{}' since it does not exist yet", room);
            create_room()
        });

        if room_state.chat_history.is_empty()
            || room_state.chat_history.len() > MAX_MESSAGES_PER_ROOM * 2
        {
            room_state.preserve_messages(&state.memory_tracker);
        }

        // Ensure we don't send more than MAX_MESSAGES_PER_ROOM on connect
        if room_state.chat_history.len() > MAX_MESSAGES_PER_ROOM {
            room_state.trim_to_max_messages(&state.memory_tracker);
        }

        let now = Instant::now();
        let (cookie_user_id, cookie_animal) = cookie
            .as_ref()
            .map(|c| (c.user_id.clone(), c.animal_name.clone()))
            .unwrap_or((String::new(), String::new()));

        // Logic: If the user_id is valid and in room_state, reuse it. Else assign new.
        let (actual_user_id, actual_animal_name) =
            if !cookie_user_id.is_empty() && !cookie_animal.is_empty() {
                if let Some(user) = room_state.users.get_mut(&cookie_user_id) {
                    // Reuse the same user ID
                    let user_id = user.user_id.clone();
                    info!(
                        "Cookie indicates existing user_id {} with animal_name {}",
                        user_id, user.animal_name
                    );

                    // If the user has a stale connection, reclaim them
                    user.connection_state = ConnectionState::Connected {
                        last_heartbeat: now,
                        connection_id: connection_id.clone(),
                    };
                    user.last_active = now;
                    user.last_message_time = now;

                    (user_id, user.animal_name.clone())
                } else {
                    info!(
                        "Cookie had user_id {}, but not found in room. Creating new user...",
                        cookie_user_id
                    );
                    let user_id = Uuid::new_v4().to_string();
                    let name = if !cookie_animal.is_empty() {
                        cookie_animal
                    } else {
                        room_state.assign_animal()
                    };

                    room_state.users.insert(
                        user_id.clone(),
                        UserData {
                            user_id: user_id.clone(),
                            animal_name: name.clone(),
                            last_active: now,
                            last_message_time: now,
                            connection_state: ConnectionState::Connected {
                                last_heartbeat: now,
                                connection_id: connection_id.clone(),
                            },
                            last_read_message: None,
                            is_typing: false,
                            last_typing_event: None,
                            last_read_receipt_event: None,
                            rate_limiter: RateLimiter::new(),
                            last_sanitized_message: None,
                        },
                    );

                    (user_id, name)
                }
            } else {
                // No cookie or invalid cookie, new user
                info!("No valid user_id cookie. Creating brand new user...");
                let user_id = Uuid::new_v4().to_string();
                let name = room_state.assign_animal();
                room_state.users.insert(
                    user_id.clone(),
                    UserData {
                        user_id: user_id.clone(),
                        animal_name: name.clone(),
                        last_active: now,
                        last_message_time: now,
                        connection_state: ConnectionState::Connected {
                            last_heartbeat: now,
                            connection_id: connection_id.clone(),
                        },
                        last_read_message: None,
                        is_typing: false,
                        last_typing_event: None,
                        last_read_receipt_event: None,
                        rate_limiter: RateLimiter::new(),
                        last_sanitized_message: None,
                    },
                );
                (user_id, name)
            };

        // Note: UserJoined will be sent in handle_websocket after subscription
        room_state.broadcast_user_count();

        // Create cookies
        let (uid_cookie, an_cookie) = create_user_cookies(&actual_user_id, &actual_animal_name);
        (actual_user_id, actual_animal_name, uid_cookie, an_cookie)
    };

    info!(
        "Assigned user_id [{}], animal_name [{}] for connection_id [{}]",
        final_user_id, final_animal_name, connection_id
    );

    let client_ip_clone = ip.clone();
    let mut res = ws
        .on_upgrade(move |socket| {
            handle_websocket(
                room,
                state,
                final_user_id,
                final_animal_name,
                socket,
                connection_id,
                client_ip_clone,
            )
        })
        .into_response();

    // Set both cookies so the user is recognized on future reloads.
    res.headers_mut()
        .append("Set-Cookie", user_id_cookie_str.parse().unwrap());
    res.headers_mut()
        .append("Set-Cookie", animal_name_cookie_str.parse().unwrap());

    Ok(res)
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum ClientEvent {
    #[serde(rename = "Message")]
    Message { text: String },
    #[serde(rename = "Typing")]
    Typing { is_typing: bool },
    #[serde(rename = "ReadReceipt")]
    ReadReceipt { message_id: String },
}

async fn handle_websocket(
    room: String,
    state: Arc<AppState>,
    user_id: String,
    animal_name: String,
    socket: WebSocket,
    connection_id: String,
    client_ip: Option<String>,
) {
    info!(
        "handle_websocket started for user_id [{}], room [{}], conn_id [{}]",
        user_id, room, connection_id
    );

    let (ws_tx, mut ws_rx) = socket.split();

    // Grab existing chat history + broadcast channel
    let (mut receiver, chat_history) = {
        let mut rooms = state.rooms.write().await;
        let room_state = match rooms.get_mut(&room) {
            Some(rs) => rs,
            None => {
                error!("Room not found: {}", room);
                return;
            }
        };

        // Ensure user is connected
        if let Some(user) = room_state.users.get_mut(&user_id) {
            user.connection_state = ConnectionState::Connected {
                last_heartbeat: Instant::now(),
                connection_id: connection_id.clone(),
            };
        } else {
            warn!(
                "User_id [{}] not found in room [{}] - creating ephemeral user data",
                user_id, room
            );
            let now = Instant::now();
            room_state.users.insert(
                user_id.clone(),
                UserData {
                    user_id: user_id.clone(),
                    animal_name: animal_name.clone(),
                    last_active: now,
                    last_message_time: now,
                    connection_state: ConnectionState::Connected {
                        last_heartbeat: now,
                        connection_id: connection_id.clone(),
                    },
                    last_read_message: None,
                    is_typing: false,
                    last_typing_event: None,
                    last_read_receipt_event: None,
                    rate_limiter: RateLimiter::new(),
                    last_sanitized_message: None,
                },
            );
            // NOTE: UserJoined broadcast is sent AFTER subscribe to ensure user receives it
        }

        let receiver = room_state.sender.subscribe();
        room_state.broadcast_user_count();

        // NOW broadcast this user's join event (after they're subscribed)
        let _ = room_state.sender.send(OutgoingEvent::System {
            event: SystemEvent::UserJoined {
                user_id: user_id.clone(),
                animal_name: animal_name.clone(),
            },
        });

        (receiver, room_state.chat_history.clone())
    };

    let ws_tx = Arc::new(Mutex::new(ws_tx));

    // Send chat history
    info!(
        "Sending {} history messages to user_id {}",
        chat_history.len(),
        user_id
    );
    {
        let ws_tx = ws_tx.clone();
        for message in chat_history.iter() {
            if let Ok(json) = serde_json::to_string(&OutgoingEvent::Message {
                message: message.clone(),
            }) {
                let mut tx = ws_tx.lock().await;
                if tx.send(Message::Text(json.into())).await.is_err() {
                    warn!(
                        "Client disconnected during history send for user_id {}",
                        user_id
                    );
                    cleanup_user(
                        &state,
                        &room,
                        &user_id,
                        &connection_id,
                        client_ip.as_deref(),
                    )
                    .await;
                    return;
                }
            }
        }
    }

    // Send a reconnect token
    let reconnect_token = Uuid::new_v4().to_string();
    info!(
        "Sending reconnect token {} to user_id {} in room {}",
        reconnect_token, user_id, room
    );
    if let Ok(json) = serde_json::to_string(&OutgoingEvent::ReconnectToken {
        token: reconnect_token,
    }) {
        let mut tx = ws_tx.lock().await;
        let _ = tx.send(Message::Text(json.into())).await;
    }

    // forward_task: broadcasted events -> this user
    let forward_task = {
        let ws_tx = ws_tx.clone();
        let user_id = user_id.clone();
        async move {
            info!("forward_task started for user_id {}", user_id);
            while let Ok(event) = receiver.recv().await {
                if let Ok(msg_json) = serde_json::to_string(&event) {
                    let mut tx = ws_tx.lock().await;
                    if tx.send(Message::Text(msg_json.into())).await.is_err() {
                        warn!("forward_task: client {} disconnected", user_id);
                        break;
                    }
                } else {
                    error!("Failed to serialize event for user {}", user_id);
                }
            }
            info!("forward_task ended for user_id {}", user_id);
        }
    };

    // receive_task: inbound messages -> handle
    let receive_task = {
        let state = state.clone();
        let room = room.clone();
        let user_id = user_id.clone();
        let animal_name = animal_name.clone();
        let ws_tx = ws_tx.clone();

        async move {
            info!("receive_task started for user_id {}", user_id);
            let mut message_count = 0;
            let mut last_msg_time = Instant::now();

            while let Some(msg_result) = ws_rx.next().await {
                let msg = match msg_result {
                    Ok(m) => m,
                    Err(e) => {
                        warn!("WebSocket receive error user_id {}: {:?}", user_id, e);
                        break;
                    }
                };
                match msg {
                    Message::Text(text) => {
                        let text_str = text.as_str();
                        if text_str.len() > MAX_PAYLOAD_SIZE {
                            warn!(
                                "Payload too large from user_id {}: {}",
                                user_id,
                                text_str.len()
                            );
                            continue;
                        }
                        debug!("Received text from user_id {}: {}", user_id, text_str);
                        let evt: Result<ClientEvent, _> = serde_json::from_str(text_str);
                        if let Ok(evt) = evt {
                            let mut rooms = state.rooms.write().await;
                            if let Some(room_state) = rooms.get_mut(&room) {
                                if let Some(user) = room_state.users.get_mut(&user_id) {
                                    user.last_active = Instant::now();
                                    if let ConnectionState::Connected {
                                        ref mut last_heartbeat,
                                        ..
                                    } = user.connection_state
                                    {
                                        // Check if heartbeat has timed out - if so, don't process messages
                                        let now = Instant::now();
                                        if now.duration_since(*last_heartbeat) > HEARTBEAT_TIMEOUT {
                                            warn!(
                                                "Heartbeat timeout for user_id {} - ignoring message",
                                                user_id
                                            );
                                            drop(rooms); // Release lock before continuing
                                            continue;
                                        }
                                        *last_heartbeat = now;
                                    }

                                    if !user.rate_limiter.can_send_message() {
                                        warn!("Rate limit exceeded for user_id {}", user_id);
                                        continue;
                                    }

                                    match evt {
                                        ClientEvent::Message { text } => {
                                            let now = Instant::now();
                                            if let Some((last_text, last_time)) =
                                                &user.last_sanitized_message
                                            {
                                                if text == *last_text
                                                    && now.duration_since(*last_time)
                                                        < SANITIZE_TIMEOUT
                                                {
                                                    warn!(
                                                        "Duplicate message from user_id {}",
                                                        user_id
                                                    );
                                                    continue;
                                                }
                                            }

                                            match validate_message(&text) {
                                                Ok(_clean_text) => {
                                                    user.last_sanitized_message =
                                                        Some((text.clone(), now));

                                                    if now.duration_since(last_msg_time)
                                                        > MESSAGE_RATE_LIMIT
                                                    {
                                                        message_count = 0;
                                                        last_msg_time = now;
                                                    }
                                                    message_count += 1;
                                                    if message_count > MAX_MESSAGES_PER_WINDOW {
                                                        warn!(
                                                            "Rate limit exceeded for user_id {}",
                                                            user_id
                                                        );
                                                        continue;
                                                    }
                                                    if text.len() > MAX_MESSAGE_LEN {
                                                        warn!(
                                                            "Message too long from user_id {}: {}",
                                                            user_id,
                                                            text.len()
                                                        );
                                                        continue;
                                                    }

                                                    let timestamp = SystemTime::now()
                                                        .duration_since(SystemTime::UNIX_EPOCH)
                                                        .unwrap_or_default()
                                                        .as_millis()
                                                        .to_string();

                                                    let md_opts = ComrakOptions::default();
                                                    let rendered_html =
                                                        markdown_to_html(&text, &md_opts);
                                                    let clean_text = ammonia::clean(&rendered_html);

                                                    let message_id = Uuid::new_v4();
                                                    let outgoing = OutgoingMessage {
                                                        message_id,
                                                        user_id: user_id.clone(),
                                                        animal_name: animal_name.clone(),
                                                        text: clean_text,
                                                        timestamp,
                                                    };

                                                    user.last_message_time = Instant::now();
                                                    room_state.add_message(
                                                        outgoing.clone(),
                                                        &state.memory_tracker,
                                                    );
                                                    let _ = room_state.sender.send(
                                                        OutgoingEvent::Message {
                                                            message: outgoing,
                                                        },
                                                    );
                                                }
                                                Err(e) => {
                                                    warn!(
                                                        "Message validation failed user_id {}: {}",
                                                        user_id, e
                                                    );
                                                    continue;
                                                }
                                            }
                                        }
                                        ClientEvent::Typing { is_typing } => {
                                            let now = Instant::now();
                                            if let Some(last) = user.last_typing_event {
                                                if now.duration_since(last)
                                                    < TYPING_EVENT_MIN_INTERVAL
                                                {
                                                    continue;
                                                }
                                            }
                                            user.last_typing_event = Some(now);
                                            let (uid, animal) = {
                                                user.is_typing = is_typing;
                                                (user.user_id.clone(), user.animal_name.clone())
                                            };

                                            room_state.broadcast_system_event(
                                                SystemEvent::Typing {
                                                    user_id: uid,
                                                    animal_name: animal,
                                                    is_typing,
                                                },
                                            );
                                        }
                                        ClientEvent::ReadReceipt { message_id } => {
                                            let now = Instant::now();
                                            if let Some(last) = user.last_read_receipt_event {
                                                if now.duration_since(last)
                                                    < READ_RECEIPT_MIN_INTERVAL
                                                {
                                                    continue;
                                                }
                                            }
                                            user.last_read_receipt_event = Some(now);
                                            if let Ok(msg_id) = Uuid::parse_str(&message_id) {
                                                let (uid, animal) = {
                                                    user.last_read_message = Some(msg_id);
                                                    (user.user_id.clone(), user.animal_name.clone())
                                                };

                                                room_state.broadcast_system_event(
                                                    SystemEvent::ReadReceipt {
                                                        user_id: uid,
                                                        animal_name: animal,
                                                        message_id: msg_id,
                                                    },
                                                );
                                            } else {
                                                warn!(
                                                    "Invalid message_id in read receipt from user_id {}: {}",
                                                    user_id, message_id
                                                );
                                            }
                                        }
                                    }
                                } else {
                                    warn!(
                                        "User_id {} not found in room {} while processing events",
                                        user_id, room
                                    );
                                }
                            } else {
                                warn!("Room {} not found while user_id {}", room, user_id);
                            }
                        } else {
                            warn!(
                                "Failed to parse client event from user_id {}: {}",
                                user_id, text_str
                            );
                        }
                    }
                    Message::Binary(_) => {
                        warn!(
                            "Unexpected binary message from user_id {}, ignoring...",
                            user_id
                        );
                    }
                    Message::Ping(payload) => {
                        debug!(
                            "Received ping from user_id {} with payload {:?}",
                            user_id, payload
                        );
                        let mut tx = ws_tx.lock().await;
                        if tx.send(Message::Pong(payload)).await.is_err() {
                            warn!("Failed to send Pong to user_id {}", user_id);
                            break;
                        }
                    }
                    Message::Pong(_) => {
                        debug!("Received pong from user_id {}", user_id);
                    }
                    Message::Close(_) => {
                        info!("Client user_id {} sent close frame", user_id);
                        break;
                    }
                }
            }
            info!("receive_task ended for user_id {}", user_id);
        }
    };

    // ping_task: keep connection alive
    let ping_task = {
        let ws_tx = ws_tx.clone();
        let user_id = user_id.clone();
        async move {
            info!("ping_task started for user_id {}", user_id);
            let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
            loop {
                interval.tick().await;
                let mut tx = ws_tx.lock().await;
                if tx.send(Message::Ping(Bytes::new())).await.is_err() {
                    warn!("ping_task: client {} disconnected", user_id);
                    break;
                }
            }
            info!("ping_task ended for user_id {}", user_id);
        }
    };

    // heartbeat_task: sends Heartbeat + updates last_heartbeat
    let heartbeat_task = {
        let state = state.clone();
        let room = room.clone();
        let user_id = user_id.clone();
        let ws_tx = ws_tx.clone();

        async move {
            info!("heartbeat_task started for user_id {}", user_id);
            let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
            loop {
                interval.tick().await;

                {
                    let mut tx = ws_tx.lock().await;
                    let beat_str =
                        serde_json::to_string(&OutgoingEvent::Heartbeat).unwrap_or_default();
                    if tx.send(Message::Text(beat_str.into())).await.is_err() {
                        warn!("heartbeat_task: client {} disconnected", user_id);
                        break;
                    }
                }

                let mut rooms = state.rooms.write().await;
                if let Some(room_state) = rooms.get_mut(&room) {
                    if let Some(user) = room_state.users.get_mut(&user_id) {
                        if let ConnectionState::Connected {
                            ref mut last_heartbeat,
                            ..
                        } = user.connection_state
                        {
                            *last_heartbeat = Instant::now();
                            debug!(
                                "Updated last_heartbeat for user_id {} in room {}",
                                user_id, room
                            );
                        }
                    }
                }
            }
            info!("heartbeat_task ended for user_id {}", user_id);
        }
    };

    tokio::select! {
        _ = forward_task => {
            info!("forward_task exited, ending session for user_id {}", user_id);
        }
        _ = receive_task => {
            info!("receive_task exited, ending session for user_id {}", user_id);
        }
        _ = ping_task => {
            info!("ping_task exited, ending session for user_id {}", user_id);
        }
        _ = heartbeat_task => {
            info!("heartbeat_task exited, ending session for user_id {}", user_id);
        }
    }

    info!(
        "All tasks ended for user_id {} in room {}. Cleaning up.",
        user_id, room
    );
    cleanup_user(
        &state,
        &room,
        &user_id,
        &connection_id,
        client_ip.as_deref(),
    )
    .await;
}

async fn cleanup_user(
    state: &Arc<AppState>,
    room: &str,
    user_id: &str,
    connection_id: &str,
    ip: Option<&str>,
) {
    info!(
        "Cleanup user_id [{}] in room [{}] for conn_id {}",
        user_id, room, connection_id
    );
    if let Some(ip) = ip {
        state.connection_pool.remove_connection(ip).await;
    }

    state
        .resource_monitor
        .total_connections
        .fetch_sub(1, Ordering::SeqCst);

    let mut rooms = state.rooms.write().await;
    if let Some(room_state) = rooms.get_mut(room) {
        if let Some(user) = room_state.users.get_mut(user_id) {
            if let ConnectionState::Connected {
                connection_id: cur_id,
                ..
            } = &user.connection_state
            {
                if cur_id == connection_id {
                    // Send "UserLeft" before switching to Disconnected
                    let _ = room_state.sender.send(OutgoingEvent::System {
                        event: SystemEvent::UserLeft {
                            user_id: user.user_id.clone(),
                            animal_name: user.animal_name.clone(),
                        },
                    });
                    user.connection_state = ConnectionState::Disconnected {
                        since: Instant::now(),
                    };
                    room_state.broadcast_user_count();
                    info!("Set user_id {} in room {} as Disconnected", user_id, room);
                } else {
                    info!(
                        "Ignoring cleanup for old conn_id {} (current is {})",
                        connection_id, cur_id
                    );
                }
            }
        } else {
            warn!(
                "User_id {} not found in room {} during cleanup",
                user_id, room
            );
        }
    } else {
        warn!("Room {} not found during cleanup user_id {}", room, user_id);
    }
}

async fn cleanup_rooms(state: &Arc<AppState>) {
    let now = Instant::now();
    let mut rooms_to_remove = Vec::new();

    // Phase 1: Identify stale disconnected users and rooms to remove (minimal lock time)
    {
        let mut rooms = state.rooms.write().await;

        for (room_name, room) in rooms.iter_mut() {
            // Clean up stale disconnected users from all rooms
            let stale_users: Vec<String> = room
                .users
                .iter()
                .filter_map(|(uid, user)| {
                    if let ConnectionState::Disconnected { since } = &user.connection_state {
                        if now.duration_since(*since) > Duration::from_secs(3600) {
                            return Some(uid.clone());
                        }
                    }
                    None
                })
                .collect();

            for uid in stale_users {
                if let Some(user) = room.users.remove(&uid) {
                    // Return animal name to pool
                    room.available_animals.push_back(user.animal_name);
                    info!(
                        "Removed stale disconnected user {} from room {}",
                        uid, room_name
                    );
                }
            }

            if room_name == "main" {
                // Main room also experiences message fade if idle
                let inactive_duration = now.duration_since(room.last_activity);
                let target_messages = if inactive_duration >= EMPTY_ROOM_CLEANUP_DELAY {
                    // If main room has been idle 1+ minute, aggressively fade: keep only ~10 recent
                    10
                } else if inactive_duration >= Duration::from_secs(30) {
                    // At 30s idle, trim down to ~100
                    100
                } else {
                    // Normal: keep up to MAX
                    MAX_MESSAGES_PER_ROOM
                };

                if room.chat_history.len() > target_messages {
                    let start_idx = room.chat_history.len().saturating_sub(target_messages);
                    let removed_bytes = room.chat_history[..start_idx]
                        .iter()
                        .map(|msg| msg.estimate_size())
                        .sum::<usize>();

                    info!(
                        "Fading main room messages from {} to {} (idle {}s)",
                        room.chat_history.len(),
                        target_messages,
                        inactive_duration.as_secs()
                    );

                    room.chat_history = room.chat_history[start_idx..].to_vec();

                    let new_total = room
                        .chat_history
                        .iter()
                        .map(|msg| msg.estimate_size())
                        .sum::<usize>();
                    room.total_memory_bytes.store(new_total, Ordering::SeqCst);

                    if removed_bytes > 0 {
                        state.memory_tracker.remove_bytes(removed_bytes);
                    }
                }
                continue;
            }

            let inactive_duration = now.duration_since(room.last_activity);

            // Check for actively connected users
            let active_users = room
                .users
                .values()
                .any(|u| matches!(u.connection_state, ConnectionState::Connected { .. }));

            // Delete empty rooms (no connected users) after EMPTY_ROOM_CLEANUP_DELAY of inactivity
            if !active_users && inactive_duration >= EMPTY_ROOM_CLEANUP_DELAY {
                rooms_to_remove.push(room_name.clone());
            }
        }

        // Phase 2: Remove identified rooms (still holding lock)
        for rm_name in &rooms_to_remove {
            info!("Removing inactive room: {}", rm_name);
            if let Some(room) = rooms.remove(rm_name) {
                let removed_bytes = room
                    .chat_history
                    .iter()
                    .map(|msg| msg.estimate_size())
                    .sum::<usize>();
                if removed_bytes > 0 {
                    state.memory_tracker.remove_bytes(removed_bytes);
                }
            }
        }
    } // Lock released here
}

#[cfg(test)]
fn generate_random_room_name() -> String {
    info!("Generating random room name...");
    let adjs = [
        "latent",
        "mellow",
        "shiny",
        "mystic",
        "curious",
        "whimsical",
        "cosmic",
        "hidden",
        "vivid",
        "serendipitous",
        "obscure",
        "nebular",
        "celestial",
        "fae",
        "ethereal",
    ];
    let nouns = [
        "toy",
        "garden",
        "forest",
        "ocean",
        "cavern",
        "nebula",
        "playground",
        "bazaar",
        "temple",
        "dojo",
        "lair",
        "grove",
        "spire",
        "oasis",
        "realm",
    ];
    let mut rng = rand::thread_rng();
    let a = adjs.choose(&mut rng).unwrap_or(&"hidden");
    let n = nouns.choose(&mut rng).unwrap_or(&"room");
    let name = format!("{}-{}", a, n);
    info!("Generated random room name: {}", name);
    name
}

/// Serves a basic `robots.txt`
async fn robots_txt_handler() -> impl IntoResponse {
    info!("Serving robots.txt");
    Response::builder()
        .header("Content-Type", "text/plain")
        .body(
            "User-agent: *\n\
            Allow: /\n\
            Disallow: /ws/\n\
            Disallow: /_*\n\
            Crawl-delay: 10\n\n\
            # Prevent access to WebSocket endpoints\n\
            Disallow: /ws/*\n"
                .to_string(),
        )
        .unwrap()
}

/// Health check endpoint for load balancers
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    connections: usize,
    rooms: usize,
    memory_bytes: usize,
}

async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let rooms = state.rooms.read().await;
    let room_count = rooms.len();
    let connections = state
        .resource_monitor
        .total_connections
        .load(Ordering::Relaxed);
    let memory = state.memory_tracker.total_bytes.load(Ordering::Relaxed);
    drop(rooms);

    Json(HealthResponse {
        status: "healthy",
        version: VERSION,
        connections,
        rooms: room_count,
        memory_bytes: memory,
    })
}

/// Prometheus-compatible metrics endpoint
async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let rooms = state.rooms.read().await;
    let room_count = rooms.len();
    let total_users: usize = rooms.values().map(|r| r.users.len()).sum();
    let total_messages: usize = rooms.values().map(|r| r.chat_history.len()).sum();
    drop(rooms);

    let connections = state
        .resource_monitor
        .total_connections
        .load(Ordering::Relaxed);
    let memory = state.memory_tracker.total_bytes.load(Ordering::Relaxed);
    let peak_memory = state.memory_tracker.peak_bytes.load(Ordering::Relaxed);
    let active_pool = state.connection_pool.active.load(Ordering::Relaxed);

    let metrics = format!(
        "# HELP chat_rooms_total Total number of active chat rooms\n\
         # TYPE chat_rooms_total gauge\n\
         chat_rooms_total {}\n\
         # HELP chat_connections_total Total active WebSocket connections\n\
         # TYPE chat_connections_total gauge\n\
         chat_connections_total {}\n\
         # HELP chat_users_total Total users across all rooms\n\
         # TYPE chat_users_total gauge\n\
         chat_users_total {}\n\
         # HELP chat_messages_total Total messages in memory\n\
         # TYPE chat_messages_total gauge\n\
         chat_messages_total {}\n\
         # HELP chat_memory_bytes Current memory usage in bytes\n\
         # TYPE chat_memory_bytes gauge\n\
         chat_memory_bytes {}\n\
         # HELP chat_memory_peak_bytes Peak memory usage in bytes\n\
         # TYPE chat_memory_peak_bytes gauge\n\
         chat_memory_peak_bytes {}\n\
         # HELP chat_connection_pool_active Active connections in pool\n\
         # TYPE chat_connection_pool_active gauge\n\
         chat_connection_pool_active {}\n",
        room_count, connections, total_users, total_messages, memory, peak_memory, active_pool
    );

    Response::builder()
        .header("Content-Type", "text/plain; version=0.0.4")
        .body(metrics)
        .unwrap()
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    info!("Starting server...");

    let app_state = Arc::new(AppState::new());

    // Periodic room cleanup
    {
        let state = app_state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(ROOM_CLEANUP_INTERVAL);
            loop {
                interval.tick().await;
                cleanup_rooms(&state).await;
            }
        });
    }

    // Memory cleanup, rate-limit checks, etc.
    {
        let state = app_state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                state.cleanup().await;
            }
        });
    }

    // Graceful shutdown on Ctrl-C
    let (tx, rx) = tokio::sync::oneshot::channel();
    let state_for_shutdown = app_state.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        info!("Shutdown signal received");
        state_for_shutdown.shutdown().await;
        let _ = tx.send(());
    });

    // Configure CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    let app = Router::new()
        .route("/", get(root_redirect))
        .route("/main", get(main_room_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/ws/{room}", get(ws_handler))
        .route("/{room}", get(room_handler))
        .route("/robots.txt", get(robots_txt_handler))
        .layer(cors)
        .with_state(app_state);

    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse::<u16>()
        .expect("PORT must be a number");
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    info!("Listening on {}", addr);
    let server = Server::bind(addr).serve(app.into_make_service_with_connect_info::<SocketAddr>());

    tokio::select! {
        result = server => {
            if let Err(e) = result {
                error!("Server error: {}", e);
            }
        }
        _ = rx => {
            info!("Server shutdown complete");
        }
    }
}

fn validate_message(text: &str) -> Result<String, ChatError> {
    // Trim whitespace first
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ChatError::InvalidMessage("Message cannot be empty".into()));
    }
    if trimmed.len() > MAX_MESSAGE_LEN {
        return Err(ChatError::InvalidMessage("Message too long".into()));
    }
    let clean_text = ammonia::clean(trimmed);
    if clean_text.trim().is_empty() {
        return Err(ChatError::InvalidMessage(
            "Message cannot be empty after sanitization".into(),
        ));
    }
    if clean_text.len() > MAX_MESSAGE_LEN {
        return Err(ChatError::InvalidMessage(
            "Sanitized message too long".into(),
        ));
    }
    Ok(clean_text)
}
