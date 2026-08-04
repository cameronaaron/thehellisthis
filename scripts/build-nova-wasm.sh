#!/bin/bash
# Rebuilds nova-wasm/pkg/ (the nova room's WASM crypto bindings) and stamps
# its output where routes/page.rs::NOVA_JS / NOVA_WASM include_str!/
# include_bytes! it from.
#
# Not run by CI and not part of `cargo build` for the main crate — same
# model as index.html's own include_str!: the built artifact is checked in,
# and this script is how you regenerate it after editing nova-wasm/src/lib.rs
# or bumping the pinned novachannel rev (Cargo.toml *and*
# nova-wasm/Cargo.toml both name the same rev; novachannel-watch.yml bumps
# both in one PR).
#
# Requires `wasm-pack` (`brew install wasm-pack` or
# `cargo install wasm-pack`) and the `wasm32-unknown-unknown` target
# (`rustup target add wasm32-unknown-unknown`).

set -euo pipefail
cd "$(dirname "$0")/../nova-wasm"

wasm-pack build --target web --release

echo
echo "Built. Verify with the two commands the CI gate itself runs:"
echo "  cd .. && cargo build && cargo test --all-features session_nova"
