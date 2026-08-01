//! Animal-name assignment: the rotation pool, and honouring a returning
//! visitor's cookie only when the name is genuinely still theirs to keep.

use std::collections::{HashMap, HashSet};

use tracing::{debug, trace};

use crate::animals::is_animal_name;

use super::{RoomState, UserData};

/// The names currently held by connected users in a room.
///
/// Authoritative: whether a name is free is *derived* from the users, never
/// read from separate bookkeeping that could drift out of step with them.
/// Takes the map rather than the whole room so callers can hold this set while
/// mutating the name pool — they are disjoint fields.
fn names_in_use(users: &HashMap<String, UserData>) -> HashSet<&str> {
    users
        .values()
        .filter(|u| u.is_connected())
        .map(|u| u.animal_name.as_str())
        .collect()
}

impl RoomState {
    /// Takes an unused name from the pool.
    ///
    /// Builds the in-use set once (O(users)) and then scans the pool once
    /// (O(pool)). The previous form re-scanned every user for every candidate
    /// name, which is O(pool × users) — with a full room and a full pool that
    /// is ~25,000 string comparisons while holding the room write lock.
    ///
    /// The pool is a **rotation cursor, not a free list**: every name drawn is
    /// pushed to the back whether or not it was handed out, so the pool is
    /// always exactly the roster in this room's own order. It used to be a free
    /// list that callers put names back into by hand, and the bookkeeping did
    /// not balance — a name that was never drawn from *this* room's pool (one
    /// carried in on a cookie, or a `guest_N` fallback) was still pushed back
    /// when its user was reclaimed, so the pool grew on every such reclaim and
    /// accumulated names that were not on the roster. Deriving "free" from
    /// `users` instead deletes the whole class: there is no second copy of the
    /// answer to get out of step.
    pub fn assign_animal(&mut self) -> String {
        // Borrows `users` rather than `self`, so the pool below stays mutable.
        let taken = names_in_use(&self.users);
        let mut chosen = None;

        // Rotate the pool at most once. `pop_front` cannot fail inside this
        // bound, so it is unwrapped rather than guarded by a branch nothing
        // could ever exercise.
        for _ in 0..self.available_animals.len() {
            let animal = self
                .available_animals
                .pop_front()
                .expect("pool length was just measured");
            let free = !taken.contains(animal.as_str());
            self.available_animals.push_back(animal);

            if free {
                chosen = self.available_animals.back().cloned();
                break;
            }
        }

        if let Some(animal) = chosen {
            trace!(animal = %animal, "assigned animal name");
            return animal;
        }

        // Every roster name is held by a connected user. Unreachable while the
        // roster is larger than `MAX_USERS_PER_ROOM`, which
        // `the_roster_is_larger_than_a_room_can_ever_be` enforces.
        let name = format!("guest_{}", self.users.len() + 1);
        debug!(name = %name, "animal pool exhausted, assigned guest name");
        name
    }

    /// The name a joining visitor should get, honouring `preferred` only if it
    /// is genuinely theirs to take.
    ///
    /// `preferred` comes from an identity cookie, which is a claim by the
    /// client and nothing more: `HttpOnly` stops a page's script from touching
    /// the cookie, but not the person driving the browser from sending any
    /// `Cookie` header they like. It is honoured only when it is on the roster
    /// *and* free in this room — so a returning visitor keeps their name, while
    /// a forged one cannot invent a display name, cannot make it a megabyte
    /// long, and cannot impersonate somebody already in the room.
    pub fn claim_animal(&mut self, preferred: Option<&str>) -> String {
        if let Some(name) = preferred
            && is_animal_name(name)
            && !names_in_use(&self.users).contains(name)
        {
            return name.to_string();
        }
        self.assign_animal()
    }

    /// Whether `name` is held by a *different*, currently connected user.
    ///
    /// `names_in_use` (and so `claim_animal`/`assign_animal`) only count a
    /// name as taken while its holder is connected — a disconnected user's
    /// entry lingers for `DISCONNECTED_USER_RETENTION` so they can reclaim
    /// their identity, but their name is free to hand to somebody else in the
    /// meantime. Reclaiming has to check this before reinstating a stored
    /// name unconditionally, or two connected users end up sharing one.
    pub fn name_taken_by_another_connected_user(&self, user_id: &str, name: &str) -> bool {
        self.users
            .iter()
            .any(|(uid, u)| uid != user_id && u.is_connected() && u.animal_name == name)
    }
}
