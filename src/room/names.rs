//! Animal-name assignment: the rotation pool, and honouring a returning
//! visitor's cookie only when the name is genuinely still theirs to keep.

use std::collections::{HashMap, HashSet};

use tracing::{debug, trace};

use crate::animals::is_animal_name;

use super::{RoomState, UserData};

/// Names held by connected users in a room: the authority on whether a name
/// is free, derived from the users rather than separate bookkeeping. Takes
/// the map, not the room, so callers can hold it while changing the name
/// pool.
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
    /// Builds the in-use set once (O(users)), then scans the pool once (O(pool)),
    /// instead of rescanning users per candidate: about 25,000 comparisons under
    /// the write lock in a full room.
    ///
    /// The pool is a rotation cursor, not a free list: every drawn name goes to
    /// the back, so the pool is always the roster in this room's order. As a free
    /// list the bookkeeping didn't balance: names never drawn here (from a
    /// cookie, or `guest_N`) were pushed back on reclaim, so the pool grew and
    /// filled with non-roster names. Deriving "free" from `users` removes the
    /// second copy (§1.4b).
    pub fn assign_animal(&mut self) -> String {
        // Borrows `users` rather than `self`, so the pool below stays mutable.
        let taken = names_in_use(&self.users);
        let mut chosen = None;

        // Rotate the pool at most once; `pop_front` can't fail within this
        // bound, so it is unwrapped rather than given an unreachable branch.
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

        // Every roster name is taken. Unreachable while the roster is larger
        // than `MAX_USERS_PER_ROOM`
        // (`the_roster_is_larger_than_a_room_can_ever_be`).
        let name = format!("guest_{}", self.users.len() + 1);
        debug!(name = %name, "animal pool exhausted, assigned guest name");
        name
    }

    /// The name a joining visitor should get, honouring `preferred` only if it is
    /// theirs to take. It comes from a cookie, a client's claim (§5.9), so it is
    /// used only when on the roster and free here: a returning visitor keeps their
    /// name, and a forged one can't invent a name, make it huge, or take someone
    /// else's.
    pub fn claim_animal(&mut self, preferred: Option<&str>) -> String {
        if let Some(name) = preferred
            && is_animal_name(name)
            && !names_in_use(&self.users).contains(name)
        {
            return name.to_string();
        }
        self.assign_animal()
    }

    /// Whether `name` belongs to a different, connected user. In-use counts only
    /// connected holders, and a disconnected user's entry lingers for
    /// `DISCONNECTED_USER_RETENTION` while their name is free to others, so
    /// reclaiming must check this before restoring a stored name.
    pub fn name_taken_by_another_connected_user(&self, user_id: &str, name: &str) -> bool {
        self.users
            .iter()
            .any(|(uid, u)| uid != user_id && u.is_connected() && u.animal_name == name)
    }
}
