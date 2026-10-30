# deer game

A deer wandering a low-poly forest at dawn, alongside everyone else's deer. The world is a
512 m square that wraps at the edges; every client and the server generate the same terrain,
lakes, trees and bushes from one seed, so other deer never clip into terrain. The server picks
a random seed when it starts (`DEER_SEED=n` fixes it) and sends it to each client as it joins. Other players are ghosts with a name tag; the only thing shared besides
poses is which berries are still on the bushes, and the wolves.

A few wolves roam the whole forest. They are the server's: it decides where they go and
who they go after, and clients draw them in the present (carried on with their last velocity,
easing onto each new snapshot), not a snapshot behind like other deer. A wolf that spots a deer barks,
gives it a second to stop and look round, and stares it down, barking; stared at for 10 s, or looked away from, it walks slowly up. Turn and
run and it chases, at a fixed speed: a deer with its heart 80% full or more outruns it, and one with less doesn't. Within about a deer's length it pounces,
and your deer is knocked flying (`knock.rs`): away from the wolf, high or far but never up into
the leaves, bouncing off trunks at any height, then lying a moment and getting up, with the heart
nearly emptied. Other players see the tumble too. The wolf then leaves deer alone for 15 s.

## Layout

- `crates/shared`: the seeded world (tileable noise, heightfield, lakes, tree/boulder/bush
  placement) and the wire protocol (bincode over WebSocket).
- `crates/server`: axum WebSocket server at `/ws`. Relays poses as 15 Hz snapshots and owns the
  bushes: validates eats, broadcasts them, and regrows a bare bush after 25 s. Runs the wolves
  (`wolf.rs`).
- `crates/client`: the Bevy 0.20 game. Native for quick iteration, WebGPU for the web.

## Running locally

```bash
cargo run -p deer-server
```

```bash
cargo run -p deer-client
```

`DEER_WOLVES=n` sets how many wolves there are (5), and `DEER_WOLF_TEST=m` puts one `m` metres in
front of each deer as it arrives (behind it, if negative), to try them out.

Run the client twice to see a ghost. Without a server the game still plays, alone, and keeps
trying to connect. `DEER_SERVER=ws://host:3560/ws` points a native client elsewhere.

Controls: WASD to walk, hold Left Shift to gallop (forward only), Space to jump, mouse to look,
click a glowing berry to eat it or the water to drink, T to say something (Enter sends, Esc
cancels). The heart in the corner fills with berries and slowly empties; a fuller heart makes a
faster deer. Stand still for a moment and the deer sniffs out the nearest deer out of sight. The deer can wade into shallow water,
up to its belly (about 70 cm). Click captures the mouse; Esc frees it.

### Web

```bash
./build-web.sh wasm-dev
```

```bash
DEER_STATIC=dist cargo run -p deer-server
```

Then open http://127.0.0.1:3560. `./build-web.sh` with no argument makes the optimised
release build (wasm-opt, gzip, brotli). Like endif-tf it builds the game twice: a WebGPU
version with the full lighting, and a WebGL2 version for browsers without WebGPU (Firefox on
Linux, for one) that lights everything in a simpler forward pass, with no screen-space
reflections, physical sky or TAA. The page downloads only the one it picks, falls back to WebGL2
if the WebGPU one fails to start, and `?backend=webgl2` or `?backend=webgpu` forces one.
`./build-web.sh wasm-dev webgl2` builds just one, quickly. `wasm-bindgen` must match the
version in `Cargo.lock`:

```bash
cargo binstall -y --install-path target/tools/bin wasm-bindgen-cli@0.2.129
```

and run `build-web.sh` with `target/tools/bin` on `PATH`.

### Debug switches (client, all environment variables)

| Variable | Effect |
|---|---|
| `DEER_SOLO=1` | Skip the title and server, straight into the world |
| `DEER_AUTOPLAY=1` | Press play on the title after 1.5 s |
| `DEER_AT=x,z`, `DEER_YAW=rad` | Where the player's deer spawns and faces |
| `DEER_NAME=...`, `DEER_STAG=1` | Name and stag without using the title screen |
| `DEER_CAM=x,y,z,tx,ty,tz` | Fixed camera |
| `DEER_SHOT=file.png`, `DEER_SHOT_SECS=s` | Render off-screen (hidden window), save a screenshot after `s` seconds (or a list `8,8.1,8.2`), quit |
| `DEER_CLICKS=t1,t2`, `DEER_PITCH=rad`, `DEER_LOOK_YAW=rad` | Scripted left clicks (as if the mouse were captured, without taking it) and view angles (the angles apply only with `DEER_CLICKS`) |
| `DEER_LOOK_AT=t,yaw,pitch` | Swing the view to these angles at `t` seconds (to see an action from the side) |
| `DEER_HOLD=w` / `DEER_HOLD=run` | Hold W (and Shift) from 3 s on |
| `DEER_SAY=t,line` | Say `line` in chat at `t` seconds |
| `DEER_KNOCK=t` | Knock the player's deer flying at `t` seconds, as a wolf would (to look at the ragdoll without waiting for one) |
| `DEER_COLLIDERS=1` | Draw the trunk collision circles near the player, and where each deer's mouth is |
| `DEER_FPS=1` | Log the frame rate |
| `DEER_PROFILE=1` | Hidden window, uncapped; print the average frame time from 10 s to 16 s, quit (use with `DEER_SHOT`) |
| `DEER_OFF=a,b` | Switch features off for profiling: shadows, ssr, taa, bloom, atmo, envlight, trees, detail, wind, beams, ambient, fog |
| `DEER_RES=WxH` | Size of the off-screen render for `DEER_SHOT` / `DEER_PROFILE` |
| `DEER_LITE=1` | Use the WebGL2 build's simpler renderer natively, to look at it without a browser |
| `DEER_SCALE=0.6`, `DEER_NO_SCALE=1` | Force the 3D render scale, or render at full resolution |
| `DEER_CASCADES=n,first,max`, `DEER_SHADOW_SIZE=px`, `DEER_LOD1=m`, `DEER_LOD2=m` | Shadow and tree-detail tuning |

When running the built binary directly instead of through cargo, set
`BEVY_ASSET_ROOT=crates/client`.

## Performance

What keeps it fast (measured with `DEER_PROFILE`, see `crates/client/src/debug.rs`):

- **Dynamic resolution** (`render_scale.rs`): the 3D view renders at no more than a
  1440p screen's worth of pixels, is sharpened (CAS) and stretched under a full-resolution UI;
  it steps down further if frames run long.
- **Shadows**: two 1024 px cascades out to 50 m. The cost is alpha-tested leaf overdraw, so
  map size matters more than anything; 3 x 2048 px cost four times as much.
- **Tree LOD** (`lod.rs`): made at load time. Bark is simplified by vertex clustering, leaves
  by keeping a fraction of the cards and growing them; whole chunks switch level by distance.
- **No per-frame material writes**: wind reads Bevy's clock from the view globals, so the
  foliage materials never change (changing them re-specialised every tree every frame).
- **Frozen sky lighting**: the atmosphere's environment map is generated once, then kept.
- **Streaming**: ground cover and berry bushes only exist near the player; small things also
  have distance cut-offs.
- **WebGL2** spends more CPU per frame than WebGPU, so its fog closes in sooner, its far plane
  is nearer and its ground cover stops at 45 m rather than 70 m. Bevy allows it only one shadow
  cascade, so that one is 2048 px and covers the whole 50 m.

## Deploying

Like awhbs: every push to `main` builds the web game and the server on GitHub's runners, and the
self-hosted `sarah-vps` runner publishes them (`.github/workflows/deploy.yml`). One-time setup on
the VPS is described at the top of `deploy/deer.nginx.conf` and `deploy/deer.supervisor.conf`.

## Credits

Every model, texture, sound and font comes from elsewhere; see [CREDITS.md](CREDITS.md).
