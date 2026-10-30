#!/usr/bin/env bash
# Builds the static web version into ./dist:
#   index.html, deer-webgpu.js / _bg.wasm (+ .gz/.br), assets-<hash>/
# WebGPU only: the lighting (deferred shading, screen-space reflections, volumetric fog, the
# atmosphere) has no WebGL2 path.
#
#   ./build-web.sh [profile]     default: wasm-release (use wasm-dev for a quick unoptimised build)
#
# Needs: rustup target wasm32-unknown-unknown, wasm-bindgen-cli matching Cargo.lock,
# optionally wasm-opt (binaryen) and brotli.
set -euo pipefail
cd "$(dirname "$0")"

PROFILE="${1:-wasm-release}"
case "$PROFILE" in wasm-release) OPTIMIZE=1 ;; *) OPTIMIZE=0 ;; esac
PY="$(command -v python3 || command -v python)"
ASSETS=crates/client/assets

# Content hash of the assets: the directory name changes whenever any asset does, so nginx can
# mark everything in it immutable.
ASSET_HASH="$("$PY" - "$ASSETS" <<'PYEOF'
import hashlib, os, sys
root = sys.argv[1]
h = hashlib.sha1()
for d, _, files in sorted(os.walk(root)):
    for f in sorted(files):
        p = os.path.join(d, f)
        h.update(os.path.relpath(p, root).replace(os.sep, "/").encode())
        h.update(b"\0")
        with open(p, "rb") as fh:
            h.update(fh.read())
print(h.hexdigest()[:10])
PYEOF
)"
export DEER_ASSET_DIR="assets-$ASSET_HASH"

WASM_OPT="${WASM_OPT:-$(command -v wasm-opt || true)}"
[ "$OPTIMIZE" = 1 ] && [ -z "$WASM_OPT" ] && echo "wasm-opt not found; skipping (the wasm will be larger)"
HAVE_BROTLI=0; command -v brotli >/dev/null 2>&1 && HAVE_BROTLI=1

STAGE="target/web-stage"
rm -rf "$STAGE"
mkdir -p "$STAGE" dist

echo "== building deer-webgpu ($PROFILE, $DEER_ASSET_DIR)"
cargo build -p deer-client --target wasm32-unknown-unknown --profile "$PROFILE" --features webgpu
wasm-bindgen --target web --no-typescript --out-dir "$STAGE" --out-name deer-webgpu \
  "target/wasm32-unknown-unknown/$PROFILE/deer-client.wasm"
if [ "$OPTIMIZE" = 1 ] && [ -n "$WASM_OPT" ]; then
  echo "running wasm-opt"
  "$WASM_OPT" -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
    --enable-mutable-globals --enable-reference-types --enable-multivalue \
    -o "$STAGE/deer-webgpu_bg.wasm" "$STAGE/deer-webgpu_bg.wasm"
fi
if [ "$OPTIMIZE" = 1 ]; then
  for f in "$STAGE/deer-webgpu_bg.wasm" "$STAGE/deer-webgpu.js"; do
    gzip -9 -k -f "$f"
    [ "$HAVE_BROTLI" = 1 ] && brotli -f -q 9 -o "$f.br" "$f"
  done
fi

WASM_BYTES="$(wc -c < "$STAGE/deer-webgpu_bg.wasm" | tr -d " ")"
CACHE_BUST="$(git rev-parse --short HEAD 2>/dev/null || echo dev)-$(date +%s)"
sed -e "s|__ASSET_DIR__|$DEER_ASSET_DIR|g" \
    -e "s|__CACHE_BUST__|$CACHE_BUST|g" \
    -e "s|__WASM_BYTES__|$WASM_BYTES|" \
    web/index.html > "$STAGE/index.html"

# assets: copy once per hash, drop older hashed copies
if [ ! -d "dist/$DEER_ASSET_DIR" ]; then
  cp -r "$ASSETS" "dist/$DEER_ASSET_DIR.tmp" && mv "dist/$DEER_ASSET_DIR.tmp" "dist/$DEER_ASSET_DIR"
  find "dist/$DEER_ASSET_DIR" \( -name 'LICENSE*' -o -name '*.md' \) -delete
fi
for d in dist/assets-*; do
  [ "$d" != "dist/$DEER_ASSET_DIR" ] && rm -rf "$d"
done
rm -f dist/deer-webgpu.js* dist/deer-webgpu_bg.wasm*
mv -f "$STAGE"/deer-* dist/
mv -f "$STAGE/index.html" dist/index.html
rm -rf "$STAGE"
echo "web build ready in ./dist ($DEER_ASSET_DIR, wasm $WASM_BYTES bytes)"
