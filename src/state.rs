//! Process-wide state shared by every connection.
//!
//! One `RwLock<HashMap<..>>` over all rooms, held as briefly as each operation
//! allows. Each method says what it does under that lock, because lock
//! duration, not CPU, is the scaling limit.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use novachannel::group::Group;
use tokio::sync::{Mutex, RwLock, broadcast};
use tracing::{debug, info};

use crate::config::{MAX_ROOMS, MAX_TOTAL_ROOMS_MEMORY, NOVA_GROUP_CAPACITY};
use crate::limits::{ConnectionPool, MemoryTracker, ResourceMonitor, SecurityManager};
use crate::protocol::{OutgoingEvent, SystemEvent, encode_broadcast};
use crate::room::{BROADCAST_CHANNEL_CAPACITY, RoomState};
use crate::session::{NovaOperatorRegistry, NovaRlnGroup};

pub struct AppState {
    pub rooms: RwLock<HashMap<String, RoomState>>,
    pub resource_monitor: ResourceMonitor,
    pub memory_tracker: MemoryTracker,
    pub connection_pool: ConnectionPool,
    pub security_manager: SecurityManager,
    /// The signing identity the server uses as the `nova` room's founding `Group`
    /// member (leaf 0), which proposes every commit, so it lives for the whole
    /// process. Regenerated each start (TOFU, as constraint #20 treats names): there
    /// is nowhere to persist a long-term key, and a restart looks like a new
    /// session to nova clients, not an error.
    pub nova_identity: novachannel::Identity,
    /// The pairwise channel's long-term DH key (X3DH's `DH1` term), unrelated to
    /// `nova_identity` (the group's signing key), and likewise regenerated each
    /// start. Broadcasts go through the `Group`; sends and single-recipient
    /// replies use this pairwise channel (`session/nova.rs`).
    pub nova_dh_identity: novachannel::prekey::DhIdentity,
    /// The signed prekey the pairwise handshake uses. No one-time prekeys: every
    /// bundle's `one_time_prekey` is `None`. Their extra forward-secrecy term
    /// guards a future compromise of both the long-term and signed-prekey secrets,
    /// which are regenerated every restart here anyway, and a depleting store
    /// would need a replenishment policy this demo doesn't have.
    pub nova_signed_prekey: novachannel::prekey::SignedPreKey,
    /// The public bundle for the two keys above, built once at startup and served
    /// as is, since it never changes during the process.
    pub nova_prekey_bundle: novachannel::prekey::PreKeyBundle,
    /// The `nova` room's shared `Group` (`session/nova.rs`): one TreeKEM-inspired
    /// tree for the room, not a ratchet per connection. Its own mutex rather than
    /// the room lock, since commits and seals are pure crypto over this state.
    /// Capacity is fixed at [`NOVA_GROUP_CAPACITY`].
    pub nova_group: Mutex<Group>,
    /// The `nova` room's sealed broadcast channel, which
    /// `session/lifecycle.rs::join_room` subscribes nova connections to instead of
    /// `RoomState::sender`. `session/nova.rs::run_seal_loop` republishes each
    /// plaintext frame here, sealed once; it must be the only broadcast caller of
    /// `Group::seal`, hence a channel of its own.
    pub nova_sealed_sender: broadcast::Sender<Arc<str>>,
    /// `nova`'s RLN membership tree and nullifier set (`session/nova_rln.rs`), held
    /// here for the one room that has it rather than on every `RoomState`.
    pub nova_rln_group: RwLock<NovaRlnGroup>,
    /// Live `nova-operator` connections and DKG ceremony state
    /// (`session/nova_operator.rs`): separate processes, whose shares this server
    /// never holds.
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
        let nova_group = Group::create(&nova_identity, NOVA_GROUP_CAPACITY)
            .expect("NOVA_GROUP_CAPACITY is a fixed power of two >= 2");
        let (nova_sealed_sender, _) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);

        let nova_pairwise_identity = novachannel::Identity::generate();
        let nova_dh_identity = novachannel::prekey::DhIdentity::generate();
        let nova_signed_prekey =
            novachannel::prekey::SignedPreKey::generate(&nova_pairwise_identity);
        let nova_prekey_bundle = novachannel::prekey::PreKeyBundle::build(
            nova_pairwise_identity.public(),
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
            nova_identity,
            nova_dh_identity,
            nova_signed_prekey,
            nova_prekey_bundle,
            nova_group: Mutex::new(nova_group),
            nova_sealed_sender,
            nova_rln_group: RwLock::new(NovaRlnGroup::new()),
            nova_operator_registry: RwLock::new(NovaOperatorRegistry::default()),
        }
    }

    /// Tells every connected user they're being disconnected, then drops all
    /// rooms. Best effort: the SIGINT grace period is short.
    pub async fn shutdown(&self) {
        info!("beginning graceful shutdown");
        let mut rooms = self.rooms.write().await;

        for room_state in rooms.values() {
            announce_departures(room_state);
        }

        rooms.clear();
        info!("shutdown complete");
    }

    /// Periodic housekeeping: expire stale address counters, and sweep history if
    /// the process is over its memory ceiling. The write lock is taken only once
    /// the cheap checks find work.
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

/// Announces every connected user in `room_state` as having left. A function
/// so "who gets told" (connected users only) can be tested on a hand-built
/// room instead of only by shutting an `AppState` down.
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
