//! Process-wide state shared by every connection.
//!
//! One `RwLock<HashMap<..>>` over all rooms, held for as short a span as the
//! operation allows. Every method here documents what it does under that lock,
//! because lock duration — not CPU — is this server's scaling limit.

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::config::{MAX_ROOMS, MAX_TOTAL_ROOMS_MEMORY};
use crate::limits::{ConnectionPool, MemoryTracker, ResourceMonitor, SecurityManager};
use crate::protocol::{OutgoingEvent, SystemEvent, encode_broadcast};
use crate::room::RoomState;
use crate::session::{NovaOperatorRegistry, NovaRlnGroup};

pub struct AppState {
    pub rooms: RwLock<HashMap<String, RoomState>>,
    pub resource_monitor: ResourceMonitor,
    pub memory_tracker: MemoryTracker,
    pub connection_pool: ConnectionPool,
    pub security_manager: SecurityManager,
    /// The DH-capable identity X3DH's `DH1` term uses
    /// (`novachannel::prekey::DhIdentity`). Generated fresh every process
    /// start, the same way `ConnectionPool`'s address hashing is keyed
    /// per-process (§5.6) — there is no database to persist a long-term
    /// key in, and every `nova` client trusts it on first connection
    /// (TOFU) rather than pinning it out of band, so a restart simply
    /// looks like a new session, not an error. The signing
    /// `novachannel::Identity` that signs `nova_signed_prekey` below and
    /// is embedded in `nova_prekey_bundle` is *not* stored here — it's
    /// only ever needed once, at construction, to build those two.
    pub nova_dh_identity: novachannel::prekey::DhIdentity,
    /// The medium-term signed prekey `session/nova.rs::handle_x3dh_init`
    /// DHs/decapsulates against. No one-time prekeys
    /// (`novachannel::prekey::OneTimePreKeyStore`) — every `nova` bundle's
    /// `one_time_prekey` is `None`; the extra forward-secrecy term they'd
    /// add protects against a *specific* future compromise of both the
    /// long-term and signed-prekey secrets together, which doesn't apply
    /// here since both are already regenerated every process restart, and
    /// a store that depletes as sessions consume entries needs a
    /// replenishment policy this proof-of-concept doesn't have one for.
    pub nova_signed_prekey: novachannel::prekey::SignedPreKey,
    /// The public half of the two keys above, built once at startup —
    /// `session/nova.rs` serves this on request rather than rebuilding it
    /// per connection, since nothing in it ever changes for the life of
    /// the process.
    pub nova_prekey_bundle: novachannel::prekey::PreKeyBundle,
    /// `nova`'s RLN membership tree and nullifier set
    /// (`session/nova_rln.rs`). Singleton state for the one room that has
    /// it, the same precedent as `nova_dh_identity` — not a field every
    /// other room's `RoomState` would carry for nothing.
    pub nova_rln_group: RwLock<NovaRlnGroup>,
    /// The live `nova-operator` connections and DKG ceremony state
    /// (`session/nova_operator.rs`) — genuinely separate processes, not
    /// simulated ones; this server never holds any of their secret shares.
    pub(crate) nova_operator_registry: RwLock<NovaOperatorRegistry>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let nova_identity = novachannel::Identity::generate();
        let nova_dh_identity = novachannel::prekey::DhIdentity::generate();
        let nova_signed_prekey = novachannel::prekey::SignedPreKey::generate(&nova_identity);
        let nova_prekey_bundle = novachannel::prekey::PreKeyBundle::build(
            nova_identity.public(),
            &nova_dh_identity,
            &nova_signed_prekey,
            None,
        );

        Self {
            // Pre-sized to the room cap: the map never has to rehash, and the
            // allocation is bounded by config rather than by traffic.
            rooms: RwLock::new(HashMap::with_capacity(MAX_ROOMS)),
            resource_monitor: ResourceMonitor::new(),
            memory_tracker: MemoryTracker::new(),
            connection_pool: ConnectionPool::new(),
            security_manager: SecurityManager::new(),
            nova_dh_identity,
            nova_signed_prekey,
            nova_prekey_bundle,
            nova_rln_group: RwLock::new(NovaRlnGroup::new()),
            nova_operator_registry: RwLock::new(NovaOperatorRegistry::default()),
        }
    }

    /// Tells every connected user they are being disconnected, then drops all
    /// rooms.
    ///
    /// Best-effort by design: the container gets a short grace period on
    /// SIGINT, and a clean "user left" is worth more to the people watching
    /// than a guaranteed one is to the process that is exiting anyway.
    pub async fn shutdown(&self) {
        info!("beginning graceful shutdown");
        let mut rooms = self.rooms.write().await;

        for room_state in rooms.values() {
            announce_departures(room_state);
        }

        rooms.clear();
        info!("shutdown complete");
    }

    /// Periodic housekeeping: expire stale IP counters, and sweep room history
    /// if the process is over its memory ceiling.
    ///
    /// The write lock is taken only once the cheap checks say there is work.
    pub async fn cleanup(&self) {
        // Every map keyed by a client address gets swept here (§3.5). Adding
        // one without adding it to this line is how the last two grew unbounded.
        self.connection_pool.cleanup_stale().await;
        self.security_manager.cleanup_stale().await;

        if !self.memory_tracker.should_gc() {
            return;
        }

        let mut rooms = self.rooms.write().await;
        let total_memory: usize = rooms
            .values()
            .map(|r| r.total_memory_bytes.load(Ordering::Relaxed))
            .sum();

        debug!(total_memory, rooms = rooms.len(), "periodic memory sweep");

        if total_memory > MAX_TOTAL_ROOMS_MEMORY {
            for room in rooms.values_mut() {
                room.trigger_cleanup(&self.memory_tracker).await;
            }
        }
    }
}

/// Announces every *connected* user in `room_state` as having left.
///
/// A function rather than the inner half of `shutdown`'s nested loop, so "who
/// gets told" — connected users only, not everyone the room has ever known —
/// is one thing to read and one thing a test can call directly against a
/// room built by hand, instead of a loop body reachable only by shutting an
/// `AppState` down.
pub(crate) fn announce_departures(room_state: &RoomState) {
    let mut announced = 0usize;
    for user in room_state.users.values() {
        if user.is_connected() {
            let _ = room_state
                .sender
                .send(encode_broadcast(&OutgoingEvent::System {
                    event: SystemEvent::UserLeft {
                        user_id: user.user_id.clone(),
                        animal_name: user.animal_name.clone(),
                    },
                }));
            announced += 1;
        }
    }
    if announced > 0 {
        debug!(announced, "announced departures for shutdown");
    }
}
