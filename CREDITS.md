# Credits

All third-party assets are CC0 (public domain) unless noted; none require attribution except the
antlers, berry and chat icons (CC BY 3.0), the glass icon (MIT) and the fonts (SIL OFL 1.1), but everyone is credited here.

## Models

| What | Author | Source | Licence |
|---|---|---|---|
| Deer (doe) and Stag, with animations | Quaternius, Ultimate Animated Animal Pack | https://poly.pizza/m/T6Cs7tmMHJ, https://poly.pizza/m/tQdzbZ1Cmw | CC0 |
| Trees, bushes, grass, ferns, flowers, mushrooms, rocks | Quaternius, Stylized Nature MegaKit | https://quaternius.com/packs/stylizednaturemegakit.html | CC0 |
| Maple trees (retextured with the MegaKit's leaves and bark) | Quaternius, Ultimate Stylized Nature | https://quaternius.com/packs/ultimatestylizednature.html (via https://poly.pizza/m/iGFtQd0PJO) | CC0 |
| Berry, mossy stump | Quaternius, Ultimate Nature Pack | https://quaternius.com/packs/ultimatenature.html | CC0 |
| Cattails, lily pads, calla, birds | Asset Quest, Free Lowpoly Pond Kit | https://opengameart.org/content/free-lowpoly-pond-kit | CC0 |
| Wolf, with animations | Quaternius, Ultimate Animated Animal Pack | https://poly.pizza/m/P1gU3Qkr9r | CC0 |
| Frog | Quaternius | https://poly.pizza/m/9Z2V8fpazF | CC0 |
| Rabbit | Čestmír Dammer (CDmir), kelgar.org | https://opengameart.org/content/rabbit-0 | CC0 |
| Butterfly | Čestmír Dammer (CDmir), kelgar.org | https://opengameart.org/content/butterfly-animated | CC0 |

The deer and wolf models are modified: every animation was given constant rest-pose channels for the
bones it did not key (Bevy blends each bone only over the clips that key it, so a bone missing
from one clip took another's pose). The originals are the files linked above; `tools/fill_rest_channels.py` makes the change.

Per-pack notes (download URLs, conversions) are in the `LICENSE-*.txt` files beside the models
in `crates/client/assets/models/`.

## Textures and icons

| What | Author | Source | Licence |
|---|---|---|---|
| Water ripple normal map | Bevy contributors | https://github.com/bevyengine/bevy (`assets/textures/water_normals.png`) | MIT / Apache-2.0 |
| Antlers icon (deer head) | Caro Asercion | https://game-icons.net (caro-asercion/deer-head) | CC BY 3.0 |
| Berry cursor icon (raspberry) | Delapouite | https://game-icons.net (delapouite/raspberry) | CC BY 3.0 |
| Heart (full and empty; outline added) | TokyoGeisha, Pixel Hearts | https://opengameart.org/content/pixel-hearts | CC0 |
| Chat icon: speech bubble, reduced to 36 x 32 pixels, with a pixel "T" and an outline added | Delapouite | https://game-icons.net (delapouite/chat-bubble) | CC BY 3.0 |
| Water cursor icon (glass-full) | Tabler Icons | https://tabler.io/icons | MIT |

## Fonts

Pixelify Sans by Stefie Justprince, https://fonts.google.com/specimen/Pixelify+Sans, SIL Open
Font License 1.1 (`crates/client/assets/fonts/OFL.txt`).

## Sounds

All CC0, from Freesound and Kenney; every file with its source, author and the part used is
listed in [crates/client/assets/audio/CREDITS.md](crates/client/assets/audio/CREDITS.md).

## Code

The lake shader is adapted from Bevy's water example (MIT / Apache-2.0).
