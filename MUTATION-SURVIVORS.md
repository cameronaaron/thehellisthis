# Mutation testing — the surviving mutants

The first complete sweep of the whole crate, so this is a baseline rather than
a report on a finished job. **`cargo mutants` is a manual sweep, not a gate**
(CLAUDE.md), and a full run takes about two hours.

Run: `cargo mutants` — or `cargo mutants -f src/room.rs` while iterating on one
file, which is minutes rather than hours.

## The run

| | |
| --- | --- |
| Tested | 496 |
| **Caught** | **375** |
| **Missed** | **57** |
| Unviable | 60 |
| Timeouts | 4 |
| Kill rate | 86.8% of viable mutants |

Recorded against the tree at `530a427`. Three files have been worked on since
and verified separately with scoped runs — `protocol.rs` 24/24 and
`cleanup.rs` 14/14 are clean, so the `cleanup.rs` rows below are already dead.
**Re-run before trusting any individual line here.**

## What a survivor means

The mutant changed the code and the whole suite still passed. That is one of
three things, and telling them apart is the work:

1. **A missing test.** Something real is unasserted. Most survivors are this.
2. **An equivalent mutant.** The change cannot alter behaviour — `if freed > 0`
   guarding a call that is already a no-op for zero. The fix is usually to
   delete the redundant code, not to write a test (§6.6d).
3. **Unreachable at the boundary.** Two spellings differ only when a duration
   is *exactly* its threshold. Some of these became reachable by passing the
   instant in (`cleanup_rooms_at`); the rest would need an injectable clock,
   which §9.4 rejects.

## Survivors by file

### src/session.rs — 18

```
src/session.rs:203:9: replace || with && in admit_user
src/session.rs:203:42: replace > with == in admit_user
src/session.rs:203:42: replace > with < in admit_user
src/session.rs:203:42: replace > with >= in admit_user
src/session.rs:203:66: replace * with + in admit_user
src/session.rs:203:66: replace * with / in admit_user
src/session.rs:207:38: replace > with >= in admit_user
src/session.rs:230:20: replace match guard room_state.users.contains_key(&c.user_id) with false in admit_user
src/session.rs:593:39: replace > with == in run_session
src/session.rs:593:39: replace > with >= in run_session
src/session.rs:669:48: replace > with >= in apply_client_event
src/session.rs:711:51: replace < with <= in apply_client_event
src/session.rs:730:27: replace > with >= in apply_client_event
src/session.rs:762:45: replace < with <= in apply_client_event
src/session.rs:779:45: replace < with == in apply_client_event
src/session.rs:779:45: replace < with > in apply_client_event
src/session.rs:779:45: replace < with <= in apply_client_event
src/session.rs:826:45: replace < with <= in apply_client_event
```

### src/limits.rs — 9

```
src/limits.rs:89:50: replace > with >= in RateLimiter::roll_window
src/limits.rs:153:21: replace > with >= in MemoryTracker::add_bytes
src/limits.rs:227:72: replace < with == in ConnectionPool::cleanup_stale
src/limits.rs:227:72: replace < with <= in ConnectionPool::cleanup_stale
src/limits.rs:290:36: replace < with <= in SecurityManager::check_ip
src/limits.rs:342:56: replace < with == in SecurityManager::cleanup_stale
src/limits.rs:342:56: replace < with <= in SecurityManager::cleanup_stale
src/limits.rs:347:67: replace < with == in SecurityManager::cleanup_stale
src/limits.rs:347:67: replace < with <= in SecurityManager::cleanup_stale
```

### src/room.rs — 9

```
src/room.rs:246:38: replace > with >= in RoomState::add_message
src/room.rs:287:38: replace - with + in RoomState::fade_oldest_attachments
src/room.rs:526:27: replace += with *= in RoomState::cleanup_messages
src/room.rs:531:20: replace > with >= in RoomState::cleanup_messages
src/room.rs:548:36: replace > with >= in RoomState::preserve_messages
src/room.rs:548:60: replace + with - in RoomState::preserve_messages
src/room.rs:556:36: replace > with >= in RoomState::trim_to_max_messages
src/room.rs:582:26: replace > with >= in RoomState::retain_newest
src/room.rs:656:60: replace > with >= in RoomState::trigger_cleanup
```

### src/routes.rs — 9

```
src/routes.rs:37:5: replace fnv1a -> u64 with 0
src/routes.rs:37:5: replace fnv1a -> u64 with 1
src/routes.rs:39:13: replace < with == in fnv1a
src/routes.rs:39:13: replace < with > in fnv1a
src/routes.rs:40:14: replace ^= with |= in fnv1a
src/routes.rs:40:14: replace ^= with &= in fnv1a
src/routes.rs:120:19: replace < with <= in room_handler
src/routes.rs:120:53: replace > with >= in room_handler
src/routes.rs:131:8: delete ! in room_handler
```

### src/validation.rs — 7

```
src/validation.rs:100:19: replace && with || in truncate_on_char_boundary
src/validation.rs:100:15: replace > with >= in truncate_on_char_boundary
src/validation.rs:188:26: replace | with ^ in decode_base64_prefix
src/validation.rs:248:30: replace > with == in sanitize_attachment
src/validation.rs:248:30: replace > with >= in sanitize_attachment
src/validation.rs:271:29: replace > with >= in sanitize_attachment
src/validation.rs:272:30: replace > with >= in sanitize_attachment
```

### src/state.rs — 4

```
src/state.rs:82:12: delete ! in AppState::cleanup
src/state.rs:92:25: replace > with == in AppState::cleanup
src/state.rs:92:25: replace > with < in AppState::cleanup
src/state.rs:92:25: replace > with >= in AppState::cleanup
```

### src/startup.rs — 1

```
src/startup.rs:134:5: replace log_server_result with ()
```

## Timeouts

A mutant that hangs usually broke a loop condition, so the suite never finishes
rather than failing. Worth treating as caught, but confirm each one.

```
src/routes.rs:42:11: replace += with *= in fnv1a
src/startup.rs:93:5: replace resolve_port -> u16 with 0
src/startup.rs:93:5: replace resolve_port -> u16 with 1
src/validation.rs:101:13: replace -= with /= in truncate_on_char_boundary
```

## Next

Work file by file with `cargo mutants -f <file>`, which is fast enough to
iterate against. The pattern so far, from the two files already cleared:

- `protocol.rs` was nine arithmetic survivors in `estimate_size` — every `+`
  could become `*` and the suite passed, because every assertion was *relative*
  ("bigger than", "smaller after a trim") and nothing checked what it computed.
- `cleanup.rs` was six: two guards with nothing behind them, three boundaries
  no test stood exactly on, and one log nobody read.

Both are worth expecting again here.
