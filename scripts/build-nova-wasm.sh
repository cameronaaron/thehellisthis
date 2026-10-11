#!/bin/bash
# Rebuilds nova-wasm/pkg/, which src/routes/page.rs embeds as NOVA_JS and
# NOVA_WASM. Not run by CI or `cargo build`: like index.html, the built
# artifact is committed, and this regenerates it after editing
# nova-wasm/src/lib.rs or bumping the novachannel pin (all three manifests
# move together; novachannel-watch.yml opens an issue listing the steps).
#
# Needs wasm-pack and the wasm32-unknown-unknown target
# (`rustup target add wasm32-unknown-unknown`).

set -euo pipefail
cd "$(dirname "$0")/../nova-wasm"

wasm-pack build --target web --release

echo
echo "Built. Verify with the two commands the CI gate itself runs:"
echo "  cd .. && cargo build && cargo test --all-features session_nova"
