#!/bin/bash
# Drives the real binary and reads its log for trouble.
#
# The suite proves the code does what it should. This proves the *process*
# does — it starts the release binary, behaves like a handful of browsers, and
# then reads the log the way an operator would, failing on the signatures of
# problems that have actually happened here:
#
#   identity churn      one visitor logged as `fresh` more than once, which is
#                       the "skink left / stinks joined" bug (§5.9a)
#   silent refusals     a refusal with no reason recorded
#   panics              a task dying and taking a feature with it
#   leaked slots        connections that never came back after everyone left
#
# Run before pushing anything that touches admission, identity or teardown.
# It takes about thirty seconds.

set -euo pipefail
cd "$(dirname "$0")/.."

PORT="${PORT:-3199}"
LOG="$(mktemp "${TMPDIR:-/tmp}/infinite-chat-smoke.XXXXXX")"
cleanup() {
    if [ -n "${SERVER_PID:-}" ]; then
        kill -INT "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    rm -f "$LOG"
}
trap cleanup EXIT

echo "Building the release binary..."
cargo build --release --quiet

echo "Starting the server on :$PORT"
RUST_LOG=info PORT="$PORT" ./target/release/infinite-chat > "$LOG" 2>&1 &
SERVER_PID=$!

# Wait for it to answer rather than sleeping a guessed amount.
for _ in $(seq 1 50); do
    if curl -fsS "http://localhost:$PORT/health" > /dev/null 2>&1; then break; fi
    sleep 0.2
done
curl -fsS "http://localhost:$PORT/health" > /dev/null || { echo "server never came up"; cat "$LOG"; exit 1; }

echo "Driving clients..."
PORT="$PORT" node scripts/smoke-clients.mjs

# Let teardown and one housekeeping tick land.
sleep 2

echo
echo "=== reading the log ==="
FAILED=0

report() {
    echo "  ✘ $1"
    FAILED=1
}

# 1. Identity churn: the same room admitting `fresh` visitors over and over is
#    one person being handed a new name each reconnect.
FRESH=$(grep -c 'outcome="fresh"' "$LOG" || true)
RECLAIMED=$(grep -c 'outcome="reclaimed"' "$LOG" || true)
echo "  admissions: fresh=$FRESH reclaimed=$RECLAIMED"
if [ "$FRESH" -gt 6 ]; then
    report "identity churn: $FRESH visitors admitted with no usable identity (§5.9a)"
fi

# 2. A refusal with no stated reason.
if grep -q 'refused' "$LOG" && ! grep -q 'refused:' "$LOG"; then
    report "a connection was refused without saying which ceiling refused it"
fi

# 3. Anything that died.
if grep -qiE 'panicked|panic at' "$LOG"; then
    report "a task panicked"
    grep -iE 'panicked|panic at' "$LOG" | head -3
fi

# 4. Slots that never came back. Everyone disconnected, so the server should be
#    holding none — this is the leak class constraint #1 is about.
CONNECTIONS=$(curl -fsS "http://localhost:$PORT/health" | sed 's/.*"connections":\([0-9]*\).*/\1/')
echo "  connections still held: $CONNECTIONS"
if [ "$CONNECTIONS" -ne 0 ]; then
    report "$CONNECTIONS connection slots were never released (constraint #1)"
fi

# 5. Every arrival should have a departure once the clients have gone.
# The message field, anchored — `grep -c admitted` also matched the older
# "session admitted" line, which said the same thing twice and is now gone.
ADMITTED=$(grep -cE ' admitted ' "$LOG" || true)
DEPARTED=$(grep -cE ' departed ' "$LOG" || true)
echo "  admitted=$ADMITTED departed=$DEPARTED"
if [ "$ADMITTED" -ne "$DEPARTED" ]; then
    report "$ADMITTED arrivals but $DEPARTED departures — somebody is still held"
fi

# A check that matched nothing proved nothing. This script reported success
# once while every grep found zero lines, because the log was full of ANSI
# escapes — which is §6.4 for shell: a test that cannot fail is worse than no
# test, and the way that happens here is silently matching the wrong shape.
if [ "$ADMITTED" -eq 0 ]; then
    report "the log checks matched nothing at all — the log format changed, so \
this script is not actually checking anything"
fi

echo
if [ "$FAILED" -eq 0 ]; then
    echo "✅ Smoke test passed."
else
    echo "❌ Smoke test found problems. Full log:"
    echo
    cat "$LOG"
    exit 1
fi
