//! Per-user rate limiting: messages and room-join attempts, each a fixed
//! window.

use std::time::Instant;

use crate::config::{MAX_MESSAGES_PER_WINDOW, MAX_ROOM_JOIN_ATTEMPTS, RATE_LIMIT_WINDOW};

/// A fixed window per user. Fixed rather than sliding on purpose: a sliding
/// window needs a timestamp per event, which is per-message allocation for a
/// limit whose exact edge behaviour nobody can perceive.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    pub window_start: Instant,
    pub message_count: usize,
    pub join_attempts: usize,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            window_start: Instant::now(),
            message_count: 0,
            join_attempts: 0,
        }
    }

    pub fn can_send_message(&mut self) -> bool {
        self.roll_window();

        if self.message_count >= MAX_MESSAGES_PER_WINDOW {
            return false;
        }

        self.message_count += 1;
        true
    }

    pub fn can_join_room(&mut self) -> bool {
        self.roll_window();

        self.join_attempts += 1;
        self.join_attempts <= MAX_ROOM_JOIN_ATTEMPTS
    }

    /// Resets both counters when the window has elapsed.
    fn roll_window(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.window_start) > RATE_LIMIT_WINDOW {
            self.window_start = now;
            self.message_count = 0;
            self.join_attempts = 0;
        }
    }
}
