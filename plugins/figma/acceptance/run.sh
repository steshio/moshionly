#!/bin/sh
# Converts the fixture designs with the plugin and checks the result with
# Moshionly's own import code: the checks behind the app's /import
# endpoint, and importing each design as new artboards, into an artboard,
# and again over itself keeping its animations. Also checks that the field
# list the Node tests use still matches core.
#
# Needs Node and Rust. Run from anywhere:
#
#   plugins/figma/acceptance/run.sh
#
# Set CARGO_TARGET_DIR to build somewhere other than acceptance/target.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

node "$here/../tools/convert-fixtures.js" "$out"
cargo run --quiet --manifest-path "$here/Cargo.toml" -- fields --check "$here/../fixtures/core-fields.json"
cargo run --quiet --manifest-path "$here/Cargo.toml" -- check "$out"/*.json
