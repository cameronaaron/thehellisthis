#!/bin/bash
# Compares the shipped client against scripts/client-inventory.toml.
#
# Prints what the manifest expects and cannot find, and what exists but is not
# listed. Adding a feature is then a diff you read rather than a list you
# retype: run this, check the "new" section is what you just built, and paste
# it in.
#
# The gate itself is `the_shipped_client_still_contains_everything_it_did`,
# which fails on the first list only — new things are fine, missing ones are the
# regression this exists to catch.

set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import re, tomllib

manifest = tomllib.load(open('scripts/client-inventory.toml','rb'))
html = open('index.html').read()
css = html.split('<style>')[1].split('</style>')[0]
js = open('client.js').read()

actual = {
    'ids': set(re.findall(r'id="([^"]+)"', html)),
    'selectors': {l[:-2].strip() for l in css.split('\n')
                  if l.endswith(' {') and not l.startswith((' ', '\t', '@'))},
    'methods': {m for m in re.findall(r'^\s{4}([a-zA-Z_][\w]*)\(', js, re.M)
                if m not in ('if', 'for', 'while', 'switch', 'catch', 'constructor', 'return')},
}

for kind in ('ids', 'selectors', 'methods'):
    expected = set(manifest[kind])
    missing = sorted(expected - actual[kind])
    added = sorted(actual[kind] - expected)
    print(f"\n=== {kind} ===")
    print(f"  missing ({len(missing)}):")
    for m in missing:
        print(f"    {m}")
    print(f"  new ({len(added)}):")
    for a in added:
        print(f"    {a}")
PY
