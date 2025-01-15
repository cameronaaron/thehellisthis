mod tests;

use axum::{
    extract::{Path, State, WebSocketUpgrade, FromRequestParts},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, RwLock, Mutex};
use uuid::Uuid;
use tracing::{info, warn, error, debug};
use comrak::{ComrakOptions, markdown_to_html};
use rand::prelude::SliceRandom;
use axum_server::Server;
use async_trait::async_trait;
use http::request::Parts;
use tower_cookies::Cookie;
use ammonia;
use lazy_static::lazy_static;
use regex::Regex;
use std::sync::atomic::{AtomicUsize, Ordering};
use thiserror::Error;
use std::sync::atomic::AtomicU64;
use std::time::SystemTime;

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
const RECONNECTION_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_RECONNECT_ATTEMPTS: u32 = 5;

const ROOM_CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);
const MAX_ROOM_AGE: Duration = Duration::from_secs(86400); 
const MAX_TOTAL_ROOMS_MEMORY: usize = 400_000_000;
const MAX_MESSAGES_PER_ROOM: usize = 500;
const MAX_MESSAGE_AGE: Duration = Duration::from_secs(86400 * 30);
const CLEANUP_BATCH_SIZE: usize = 100;

// Memory + reconnection
const ESTIMATED_MESSAGE_SIZE: usize = 1024; 
const RECONNECT_BACKOFF_BASE: Duration = Duration::from_secs(5);

// Rate limiting
const MAX_CONCURRENT_CONNECTIONS_PER_IP: usize = 3;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);
const MAX_ROOM_JOIN_ATTEMPTS: usize = 10;
const SANITIZE_TIMEOUT: Duration = Duration::from_millis(50);
const MAX_PAYLOAD_SIZE: usize = 512 * 1024;

// Per-user constraints
const MAX_USER_ID_LEN: usize = 36; // Typical UUID length
const MAX_CONCURRENT_USERS: usize = 400;
const MAX_MEMORY_PER_USER: usize = 1024 * 1024; // 1MB per user
const MAX_BACKOFF_ATTEMPTS: u32 = 3;
const BACKOFF_BASE_MS: u64 = 1000;

#[derive(Debug)]
struct RateLimiter {
    window_start: Instant,
    message_count: usize,
    join_attempts: usize,
    burst_allowance: usize,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            window_start: Instant::now(),
            message_count: 0,
            join_attempts: 0,
            burst_allowance: 3,
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
        self.join_attempts += 1;
        self.join_attempts <= MAX_ROOM_JOIN_ATTEMPTS
    }

    fn check_rate_limit(&mut self) -> Result<(), ChatError> {
        let now = Instant::now();
        if now.duration_since(self.window_start) > RATE_LIMIT_WINDOW {
            self.reset_counts(now);
            Ok(())
        } else if self.message_count >= MAX_MESSAGES_PER_WINDOW {
            if self.burst_allowance > 0 {
                self.burst_allowance -= 1;
                Ok(())
            } else {
                Err(ChatError::RateLimitError("Too many messages".into()))
            }
        } else {
            self.message_count += 1;
            Ok(())
        }
    }

    fn reset_counts(&mut self, now: Instant) {
        self.window_start = now;
        self.message_count = 0;
        self.burst_allowance = 3;
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

/// Parse both `user_id` and `animal_name` cookies from the request (if present).
#[async_trait]
impl<S> FromRequestParts<S> for UserCookie
where
    S: Send + Sync,
{
    type Rejection = ();

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, ()> {
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
                return Ok(UserCookie {
                    user_id: u,
                    animal_name: a,
                });
            }
        }

        info!("No valid user_id and animal_name cookies found, treating as new user.");
        Err(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct OutgoingMessage {
    message_id: Uuid,
    user_id: String,      // ** Added user_id to differentiate server-side
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
    UserJoined { user_id: String, animal_name: String },
    UserLeft { user_id: String, animal_name: String },
    Typing { user_id: String, animal_name: String, is_typing: bool },
    ReadReceipt { user_id: String, animal_name: String, message_id: Uuid },
    ServerShutdown { reason: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
enum OutgoingEvent {
    Message {
        message: OutgoingMessage,
    },
    System {
        event: SystemEvent,
    },
    UserCount {
        count: usize,
    },
    Heartbeat,
    ReconnectToken { token: String },
}

#[derive(Serialize, Deserialize)]
struct IncomingMessage {
    text: String,
}

#[derive(Serialize, Deserialize)]
struct IncomingTyping {
    is_typing: bool,
}

#[derive(Serialize, Deserialize)]
struct IncomingReadReceipt {
    message_id: Uuid,
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
        attempts: u32,
        last_connection_id: String,
    },
}

impl ConnectionState {
    fn attempts(&self) -> u32 {
        match self {
            ConnectionState::Connected { .. } => 0,
            ConnectionState::Disconnected { attempts, .. } => *attempts,
        }
    }

    fn update_heartbeat(&mut self) -> Result<(), ChatError> {
        match self {
            ConnectionState::Connected { last_heartbeat, .. } => {
                *last_heartbeat = Instant::now();
                Ok(())
            }
            _ => Err(ChatError::ConnectionError("Not connected".into())),
        }
    }

    fn is_stale(&self) -> bool {
        match self {
            ConnectionState::Connected { last_heartbeat, .. } => {
                last_heartbeat.elapsed() > HEARTBEAT_TIMEOUT
            }
            ConnectionState::Disconnected { since, .. } => {
                since.elapsed() > RECONNECTION_TIMEOUT
            }
        }
    }
}

struct UserData {
    user_id: String,
    animal_name: String,
    last_active: Instant,
    last_message_time: Instant,
    connection_state: ConnectionState,
    connection_id: String,
    last_read_message: Option<Uuid>,
    is_typing: bool,
    rate_limiter: RateLimiter,
    last_sanitized_message: Option<(String, Instant)>,
}

struct RoomState {
    created_at: Instant,
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
        self.total_bytes.fetch_sub(bytes, Ordering::SeqCst);
    }

    fn should_gc(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let last = self.last_gc.load(Ordering::Relaxed);
        now - last > 300 // GC every 5 minutes
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
            return Err(ChatError::ResourceLimit("Too many connections from IP".into()));
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
        "dog","cat","lion","tiger","elephant","giraffe","koala","penguin","panda","dolphin",
        "whale","bear","wolf","zebra","fox","owl","rabbit","kangaroo","monkey","snake",
        "parrot","cheetah","jaguar","lynx","otter","seal","peacock","sparrow","crow","hedgehog",
        "flamingo","shark","stingray","starfish","octopus","seahorse","crab","lobster","squid",
        "antelope","badger","bison","buffalo","camel","chameleon","crocodile","eagle","ferret",
        "gecko","gorilla","heron","hyena","ibis","iguana","lemur","leopard","manatee","mole",
        "moose","narwhal","newt","ostrich","platypus","porcupine","raven","salamander","sloth",
        "stork","tapir","toad","turkey","vulture","wallaby","walrus","wolverine","yak","hippo",
        "rhino","anteater","armadillo","beaver","butterfly","cormorant","coyote","dingo","dragonfly",
        "firefly","grasshopper","hamster","honeyeater","hummingbird","kingfisher","ladybug","llama",
        "meerkat","moth","ox","puffin","quail","ringtail","swan","tortoise","turtle","woodpecker",
        "wombat","orangutan","seal","manta-ray","crow","robin","grasshopper","musk-ox",
        "kiwi","harpy-eagle","peafowl","margay","capybara","squid","urchin","bandicoot","guinea-pig",
        "axolotl","dugong","fennec-fox","lynx","pika","tamarin","aardwolf","colugo","dhole","galago",
    ];
    animals.shuffle(&mut rand::thread_rng());
    let (tx, _) = broadcast::channel::<OutgoingEvent>(1000);

    RoomState {
        created_at: Instant::now(),
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

    fn add_message(&mut self, msg: OutgoingMessage) {
        self.last_activity = Instant::now();
        let msg_size = msg.estimate_size();

        let current_memory = self.total_memory_bytes.load(Ordering::Relaxed);
        if current_memory + msg_size > MAX_TOTAL_ROOMS_MEMORY {
            self.prune_old_messages(msg_size);
        }

        self.total_memory_bytes.fetch_add(msg_size, Ordering::SeqCst);
        self.chat_history.push(msg);
    }

    fn prune_old_messages(&mut self, needed_space: usize) {
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
    }

    fn broadcast_user_count(&self) {
        let now = Instant::now();
        let connected_count = self
            .users
            .values()
            .filter(|u| {
                match &u.connection_state {
                    ConnectionState::Connected { last_heartbeat, .. } => {
                        now.duration_since(*last_heartbeat) <= HEARTBEAT_TIMEOUT
                    }
                    _ => false,
                }
            })
            .count();

        info!("Broadcasting user count: {}", connected_count);
        let _ = self
            .sender
            .send(OutgoingEvent::UserCount { count: connected_count });
    }

    fn broadcast_system_event(&self, event: SystemEvent) {
        info!("Broadcasting system event: {:?}", event);
        let _ = self.sender.send(OutgoingEvent::System { event });
    }

    async fn cleanup_messages(&mut self, now: Instant) {
        let mut removed = 0;
        let mut messages_to_retain = Vec::new();

        for msg in self.chat_history.iter() {
            if removed >= CLEANUP_BATCH_SIZE {
                messages_to_retain.push(msg.clone());
                continue;
            }

            if let Ok(ts) = msg.timestamp.parse::<u128>() {
                let age = now.duration_since(Instant::now() - Duration::from_millis(ts as u64));
                if age <= MAX_MESSAGE_AGE {
                    messages_to_retain.push(msg.clone());
                } else {
                    removed += 1;
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
        }
    }

    async fn cleanup_users(&mut self, now: Instant) -> Result<Vec<(String, String)>, &'static str> {
        let mut removed_users = Vec::new();
        let mut users_to_remove = Vec::new();

        for (user_id, user) in &mut self.users {
            let remove_due_to_inactivity = now.duration_since(user.last_message_time) > INACTIVE_TIMEOUT;
            let should_remove = match &user.connection_state {
                ConnectionState::Connected { last_heartbeat, .. } => {
                    remove_due_to_inactivity || now.duration_since(*last_heartbeat) > HEARTBEAT_TIMEOUT
                }
                ConnectionState::Disconnected { since, attempts, .. } => {
                    now.duration_since(*since) > RECONNECTION_TIMEOUT
                        || *attempts > MAX_RECONNECT_ATTEMPTS
                        || remove_due_to_inactivity
                }
            };
            if should_remove {
                users_to_remove.push(user_id.clone());
                removed_users.push((user_id.clone(), user.animal_name.clone()));
            }
        }

        for user_id in users_to_remove {
            self.users.remove(&user_id);
        }

        Ok(removed_users)
    }

    fn get_reconnection_timeout(&self, attempts: u32) -> Duration {
        let backoff = RECONNECT_BACKOFF_BASE * 2u32.pow(attempts);
        std::cmp::min(backoff, RECONNECTION_TIMEOUT)
    }

    fn preserve_messages(&mut self) {
        if self.chat_history.len() > MAX_MESSAGES_PER_ROOM + 100 {
            let start_idx = self.chat_history.len() - MAX_MESSAGES_PER_ROOM;
            self.chat_history = self.chat_history[start_idx..].to_vec();

            let new_total = self
                .chat_history
                .iter()
                .map(|msg| msg.estimate_size())
                .sum::<usize>();
            self.total_memory_bytes.store(new_total, Ordering::SeqCst);
        }
    }

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

    fn update_memory_usage(&mut self) {
        let new_total = self
            .chat_history
            .iter()
            .map(|msg| msg.estimate_size())
            .sum::<usize>()
            + self.users.len() * MAX_MEMORY_PER_USER;

        self.total_memory_bytes.store(new_total, Ordering::SeqCst);
    }

    fn prune_message_queue(&mut self) {
        while self.total_memory_bytes.load(Ordering::Relaxed) > MAX_TOTAL_ROOMS_MEMORY {
            if !self.chat_history.is_empty() {
                self.chat_history.remove(0);
                self.update_memory_usage();
            } else {
                break;
            }
        }
    }

    fn broadcast_with_retry(&self, event: OutgoingEvent) -> Result<(), &'static str> {
        for attempt in 0..MAX_BACKOFF_ATTEMPTS {
            match self.sender.send(event.clone()) {
                Ok(_) => return Ok(()),
                Err(_) if attempt < MAX_BACKOFF_ATTEMPTS - 1 => {
                    tokio::task::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(
                            BACKOFF_BASE_MS * 2u64.pow(attempt),
                        ))
                        .await;
                    });
                    continue;
                }
                Err(_) => return Err("Failed to broadcast after retries"),
            }
        }
        Err("Broadcast failed")
    }

    async fn trigger_cleanup(&mut self) {
        if self.total_memory_bytes.load(Ordering::Relaxed) > (MAX_TOTAL_ROOMS_MEMORY * 9) / 10 {
            self.cleanup_messages(Instant::now()).await;
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
                    room.trigger_cleanup().await;
                }
            }
        }
    }

    async fn graceful_shutdown(&self) {
        info!("Starting graceful shutdown...");
        let mut rooms = self.rooms.write().await;
        for (room_name, room) in rooms.iter_mut() {
            info!("Shutting down room: {}", room_name);
            let shutdown_msg = OutgoingEvent::System {
                event: SystemEvent::ServerShutdown {
                    reason: "Server maintenance".into(),
                },
            };
            let _ = room.sender.send(shutdown_msg);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        info!("Graceful shutdown complete");
    }
}

/// Creates two cookies for a user: `user_id` and `animal_name`.
fn create_user_cookies(user_id: &str, name: &str) -> (String, String) {
    info!(
        "Creating cookies for user_id [{}] with animal_name [{}]",
        user_id, name
    );

    let user_id_cookie = format!(
        "user_id={}; Path=/; Max-Age={}; SameSite=Strict; HttpOnly; Secure",
        user_id,
        INACTIVE_TIMEOUT.as_secs()
    );
    let animal_name_cookie = format!(
        "animal_name={}; Path=/; Max-Age={}; SameSite=Strict; HttpOnly; Secure",
        name,
        INACTIVE_TIMEOUT.as_secs()
    );
    (user_id_cookie, animal_name_cookie)
}

async fn root_redirect() -> Redirect {
    info!("Received request at '/', redirecting to '/main'");
    Redirect::to("/main")
}

/// Serves the main chat page at /main
async fn main_room_handler() -> impl IntoResponse {
    info!("HTTP request for main room");
    let html = include_str!("../index.html");
    Html(html.to_string())
}

/// Serves the dynamic room page
async fn room_handler(Path(room): Path<String>, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    info!("HTTP request for room: {}", room);

    let reserved_paths = ["robots.txt", "sitemap.xml", "favicon.ico", ".well-known", "main", "admin", "api"];
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
        || !room.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
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
    if !text.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err("Input contains invalid characters");
    }
    Ok(())
}

/// Upgrades the connection to WebSocket for the specified room
/// Returns `Response` on success, or `Html` error on failure.
async fn ws_handler(
    Path(room): Path<String>,
    State(state): State<Arc<AppState>>,
    cookie: Option<UserCookie>,
    ws: WebSocketUpgrade,
    ip: Option<String>,
) -> Result<Response, Html<String>> {
    info!("WebSocket upgrade request for room: {}", room);

    if let Some(ip) = &ip {
        if let Err(e) = state.security_manager.check_ip(ip).await {
            return Err(Html(e.to_string()));
        }
        if !state.connection_pool.can_accept(ip).await {
            return Err(Html("Too many connections from your IP".to_string()));
        }
        if let Err(e) = state.connection_pool.add_connection(ip).await {
            return Err(Html(e.to_string()));
        }
    }

    if !state.resource_monitor.can_accept_connection() {
        return Err(Html("Server is at capacity".to_string()));
    }

    if let Err(e) = validate_input(&room, MAX_ROOM_NAME_LEN) {
        return Err(Html(e.to_string()));
    }

    state
        .resource_monitor
        .total_connections
        .fetch_add(1, Ordering::SeqCst);

    {
        let mut rooms = state.rooms.write().await;
        if let Some(room_state) = rooms.get_mut(&room) {
            if !room_state.is_user_allowed("") {
                error!("Room {} connection limit reached", room);
                return Err(Html("Room connection limit reached".to_string()));
            }
        }
    }

    let connection_id = Uuid::new_v4().to_string();
    info!("New WebSocket connection ID: {}", connection_id);

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
                return Err(Html("Room connection limit reached".to_string()));
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
            room_state.preserve_messages();
        }

        let now = Instant::now();
        let (cookie_user_id, cookie_animal) = cookie
            .as_ref()
            .map(|c| (c.user_id.clone(), c.animal_name.clone()))
            .unwrap_or((String::new(), String::new()));

        // Logic: If the user_id is valid and in room_state, reuse it. Else assign new.
        let (actual_user_id, actual_animal_name) = if !cookie_user_id.is_empty() && !cookie_animal.is_empty() {
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
                info!("Cookie had user_id {}, but not found in room. Creating new user...", cookie_user_id);
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
                        connection_id: connection_id.clone(),
                        last_read_message: None,
                        is_typing: false,
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
                    connection_id: connection_id.clone(),
                    last_read_message: None,
                    is_typing: false,
                    rate_limiter: RateLimiter::new(),
                    last_sanitized_message: None,
                },
            );
            (user_id, name)
        };

        // Send user joined event
        let _ = room_state.sender.send(OutgoingEvent::System {
            event: SystemEvent::UserJoined {
                user_id: actual_user_id.clone(),
                animal_name: actual_animal_name.clone(),
            },
        });
        room_state.broadcast_user_count();

        // Create cookies
        let (uid_cookie, an_cookie) = create_user_cookies(&actual_user_id, &actual_animal_name);
        (
            actual_user_id,
            actual_animal_name,
            uid_cookie,
            an_cookie,
        )
    };

    info!(
        "Assigned user_id [{}], animal_name [{}] for connection_id [{}]",
        final_user_id, final_animal_name, connection_id
    );

    let mut res = ws
        .on_upgrade(move |socket| {
            handle_websocket(
                room,
                state,
                final_user_id,
                final_animal_name,
                socket,
                connection_id,
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
                    connection_id: connection_id.clone(),
                    last_read_message: None,
                    is_typing: false,
                    rate_limiter: RateLimiter::new(),
                    last_sanitized_message: None,
                },
            );
            let _ = room_state.sender.send(OutgoingEvent::System {
                event: SystemEvent::UserJoined {
                    user_id: user_id.clone(),
                    animal_name: animal_name.clone(),
                },
            });
        }
        room_state.broadcast_user_count();

        (room_state.sender.subscribe(), room_state.chat_history.clone())
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
                if tx.send(Message::Text(json)).await.is_err() {
                    warn!("Client disconnected during history send for user_id {}", user_id);
                    cleanup_user(&state, &room, &user_id, &connection_id, None).await;
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
        token: reconnect_token
    }) {
        let mut tx = ws_tx.lock().await;
        let _ = tx.send(Message::Text(json)).await;
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
                    if tx.send(Message::Text(msg_json)).await.is_err() {
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
                        if text.len() > MAX_PAYLOAD_SIZE {
                            warn!("Payload too large from user_id {}: {}", user_id, text.len());
                            continue;
                        }
                        debug!("Received text from user_id {}: {}", user_id, text);
                        let evt: Result<ClientEvent, _> = serde_json::from_str(&text);
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
                                        *last_heartbeat = Instant::now();
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
                                                    warn!("Duplicate message from user_id {}", user_id);
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
                                                        warn!("Rate limit exceeded for user_id {}", user_id);
                                                        continue;
                                                    }
                                                    if text.len() > MAX_MESSAGE_LEN {
                                                        warn!("Message too long from user_id {}: {}", user_id, text.len());
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
                                                    room_state.add_message(outgoing.clone());
                                                    let _ = room_state
                                                        .sender
                                                        .send(OutgoingEvent::Message {
                                                            message: outgoing,
                                                        });
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
                                            user.is_typing = is_typing;
                                            room_state.broadcast_system_event(SystemEvent::Typing {
                                                user_id: user.user_id.clone(),
                                                animal_name: user.animal_name.clone(),
                                                is_typing,
                                            });
                                        }
                                        ClientEvent::ReadReceipt { message_id } => {
                                            if let Ok(msg_id) = Uuid::parse_str(&message_id) {
                                                user.last_read_message = Some(msg_id);
                                                room_state.broadcast_system_event(
                                                    SystemEvent::ReadReceipt {
                                                        user_id: user.user_id.clone(),
                                                        animal_name: user.animal_name.clone(),
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
                                user_id, text
                            );
                        }
                    }
                    Message::Binary(_) => {
                        warn!("Unexpected binary message from user_id {}, ignoring...", user_id);
                    }
                    Message::Ping(payload) => {
                        debug!("Received ping from user_id {} with payload {:?}", user_id, payload);
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
                if tx.send(Message::Ping(vec![])).await.is_err() {
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
                    if tx.send(Message::Text(beat_str)).await.is_err() {
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
                            debug!("Updated last_heartbeat for user_id {} in room {}", user_id, room);
                        }
                    }
                }
            }
            info!("heartbeat_task ended for user_id {}", user_id);
        }
    };

    tokio::select! {
        _ = forward_task => (),
        _ = receive_task => (),
        _ = ping_task => (),
        _ = heartbeat_task => (),
    }

    info!("All tasks ended for user_id {} in room {}. Cleaning up.", user_id, room);
    cleanup_user(&state, &room, &user_id, &connection_id, None).await;
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
            if let ConnectionState::Connected { connection_id: cur_id, .. } = &user.connection_state {
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
                        attempts: 0,
                        last_connection_id: connection_id.to_string(),
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
            warn!("User_id {} not found in room {} during cleanup", user_id, room);
        }
    } else {
        warn!("Room {} not found during cleanup user_id {}", room, user_id);
    }
}

async fn cleanup_rooms(state: &Arc<AppState>) {
    let mut rooms = state.rooms.write().await;
    let now = Instant::now();
    let mut rooms_to_remove = Vec::new();

    for (room_name, room) in rooms.iter_mut() {
        if room_name == "main" {
            // Just trim the main room
            if room.chat_history.len() > MAX_MESSAGES_PER_ROOM + 1 {
                info!(
                    "Trimming main room messages from {} to {}",
                    room.chat_history.len(),
                    MAX_MESSAGES_PER_ROOM
                );
                let start_idx = room.chat_history.len() - MAX_MESSAGES_PER_ROOM;
                room.chat_history = room.chat_history[start_idx..].to_vec();

                let new_total = room
                    .chat_history
                    .iter()
                    .map(|msg| msg.estimate_size())
                    .sum::<usize>();
                room.total_memory_bytes.store(new_total, Ordering::SeqCst);
            }
            continue;
        }

        let inactive_duration = now.duration_since(room.last_activity);
        if inactive_duration >= Duration::from_secs(7200) && room.users.is_empty() {
            rooms_to_remove.push(room_name.clone());
        }
    }

    for rm_name in rooms_to_remove {
        info!("Removing inactive room: {}", rm_name);
        rooms.remove(&rm_name);
    }
}

fn generate_random_room_name() -> String {
    info!("Generating random room name...");
    let adjs = [
        "latent", "mellow", "shiny", "mystic", "curious", "whimsical", "cosmic", "hidden", "vivid",
        "serendipitous", "obscure", "nebular", "celestial", "fae", "ethereal",
    ];
    let nouns = [
        "toy", "garden", "forest", "ocean", "cavern", "nebula", "playground", "bazaar", "temple",
        "dojo", "lair", "grove", "spire", "oasis", "realm",
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

    let app = Router::new()
        .route("/", get(root_redirect))
        .route("/main", get(main_room_handler))
        .route(
            "/ws/:room",
            get(|path, state, cookie: Option<UserCookie>, ws| async move {
                // We inject `Option<String>` for IP if you want to track IP
                let ip_addr = None;
                match ws_handler(path, state, cookie, ws, ip_addr).await {
                    Ok(response) => response,
                    Err(html) => html.into_response(),
                }
            }),
        )
        .route("/:room", get(room_handler))
        .route("/robots.txt", get(robots_txt_handler))
        .with_state(app_state);

    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse::<u16>()
        .expect("PORT must be a number");
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    info!("Listening on {}", addr);
    let server = Server::bind(addr).serve(app.into_make_service());

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
    if text.is_empty() {
        return Err(ChatError::InvalidMessage("Message cannot be empty".into()));
    }
    if text.len() > MAX_MESSAGE_LEN {
        return Err(ChatError::InvalidMessage("Message too long".into()));
    }
    let clean_text = ammonia::clean(&text);
    if clean_text.len() > MAX_MESSAGE_LEN {
        return Err(ChatError::InvalidMessage("Sanitized message too long".into()));
    }
    Ok(clean_text)
}

async fn handle_connection_error(
    state: &Arc<AppState>,
    room: &str,
    user_id: &str,
    error: &str,
) -> Result<(), &'static str> {
    let mut rooms = state.rooms.write().await;
    if let Some(room_state) = rooms.get_mut(room) {
        let (animal_name, connection_id) = if let Some(user) = room_state.users.get(user_id) {
            (user.animal_name.clone(), user.connection_id.clone())
        } else {
            return Ok(());
        };
        if let Some(user) = room_state.users.get_mut(user_id) {
            let attempts = user.connection_state.attempts();
            user.connection_state = ConnectionState::Disconnected {
                since: Instant::now(),
                attempts: attempts + 1,
                last_connection_id: connection_id,
            };
        }
        let _ = room_state.broadcast_with_retry(OutgoingEvent::System {
            event: SystemEvent::UserLeft {
                user_id: user_id.to_string(),
                animal_name,
            },
        })?;
        room_state.broadcast_user_count();
    }
    Ok(())
}
