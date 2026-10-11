# Mutation testing: the surviving mutants

`cargo mutants` is a manual sweep, not a gate; a full run takes about two
hours. Iterate on one file with `cargo mutants -f src/room/mod.rs`.

## The last full run

| | |
| --- | --- |
| Tested | 496 |
| **Caught** | **375** |
| **Missed** | **57** |
| Unviable | 60 |
| Timeouts | 4 |
| Kill rate | 86.8% of viable mutants |

That run predates the split of `session.rs`, `limits.rs`, `room.rs`,
`routes.rs`, `validation.rs` and `security.rs` into directories, so its
per-line list no longer points anywhere and is not kept here. Several of its
classes have since been fixed: `fnv1a`'s six survivors (§6.6e), the
`elapsed()` boundaries in the limiters (§6.6f), and the `protocol.rs` (24/24)
and `cleanup.rs` (14/14) files, verified by scoped runs. **A fresh full run is
the next step**; record its survivors here by file.

Where the 57 were:

| File (then) | Survivors | Shape |
| --- | --- | --- |
| `session.rs` | 18 | admission arithmetic and match guards in `admit_user`; throttle boundaries in `apply_client_event` |
| `limits.rs` | 9 | retention and window boundaries (`<` vs `<=`) |
| `room.rs` | 9 | trim and fade thresholds, one `+=` in `cleanup_messages` |
| `routes.rs` | 9 | `fnv1a` (since pinned to test vectors) and `room_handler`'s length checks |
| `validation.rs` | 7 | `truncate_on_char_boundary`, base64 prefix decoding, attachment ceilings |
| `state.rs` | 4 | the `cleanup` gate |
| `startup.rs` | 1 | `log_server_result` replaced with `()` |

The four timeouts (`fnv1a`'s `+=`, `resolve_port` returning 0 or 1, and a
`-=` in `truncate_on_char_boundary`) are probably caught, since a hang usually
means a broken loop condition, but confirm each.

## What a survivor means

The code changed and the suite still passed. That is one of:

1. **A missing test**: something real is unasserted. Most survivors.
2. **An equivalent mutant**: the change can't alter behaviour (`if freed > 0`
   guarding a call that is already a no-op at zero). Usually the fix is
   deleting the redundant code (§6.6d).
3. **Unreachable at the boundary**: two spellings differ only at exactly a
   duration threshold. Passing the instant in (`cleanup_rooms_at`) made some
   reachable; the rest need an injectable clock, which §9.4 rejects.

## What to expect

From the files already cleared: `protocol.rs` had nine arithmetic survivors
in `estimate_size` (every `+` could become `*`), because its assertions were
all relative ("bigger than", "smaller after a trim"). `cleanup.rs` had six:
two guards with nothing behind them, three boundaries no test stood on, and a
log nobody read. Expect both shapes again.
