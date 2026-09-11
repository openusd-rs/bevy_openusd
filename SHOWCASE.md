# Composed animation showcase

Run from the repository root; every dependency is bundled in assets/:

```sh
USD_VIEWER_PANE=timeline USD_VIEWER_DOME=/Showcase/Environment \
  make run CARGO='cargo --offline' ARGS=assets/flagship_showcase.usda
```

Press **Play**, drag the **Time** scrubber, or enter 0, 30 or 60 in **USD time code** and click **Seek and
pause**. The range is 0–60 at 24 time codes per second. Open **Lighting** and
choose **Use studio only** if the device cannot filter dome maps.

The UI dependency uses `../mara-bevy-host/mara`: Mara `develop` commit `b792f44`
plus local host fixes, at `15a2e9d` on branch `fix/bevy-host-develop`.
The original `../mara` checkout remains untouched. This separate worktree is
required to build; its commits have not been pushed upstream. Whole-window GPU
screenshots and embedded dome filtering are restored. If dome
filtering reports an error, the viewer temporarily restores studio lighting
and displays the error with a fallback notice in Lighting. The requested dome
remains selected; studio fallback ends when dome maps become available.
Playback uses the host's active viewport rate without requiring mouse movement;
paused scenes retain idle throttling.

| Location | What to inspect |
| --- | --- |
| Back left | Cube grows from size 1 to 2 |
| Back row | Three instances share a growing tetrahedron prototype |
| Front left | Two-joint bar bends at 30 and returns upright at 60 |
| Front middle | Morph panel moves one corner out of plane |
| Front right | Metallic sphere; red/blue reflections require a dome-capable host |
| Animated objects | Shared material changes orange to blue |

The scene references animation_showcase.usda with time scale 6, the skeleton
fixture directly, and morph_animation.usda with time scale 6. Local overrides
place and bind the composed objects without editing their source layers. The
example contract checks retimed values and remapped relationships:

```sh
make test CARGO='cargo --offline' APP_TARGET='--example showcase_contract'
```

Use the **Outliner** to select an object, then **Inspector** to examine its
composed values and source opinions. Root-layer save retains composition;
flattened export is a different operation. See [SUPPORT.md](SUPPORT.md) for the
capability/approximation matrix and [PACKAGING.md](PACKAGING.md) for portable
USDZ export boundaries.

Direct whole-viewer captures at 0, 30 and 60 verify visible growth, deformation,
material changes and the Timeline values. This assembled fixture is not a native
reference-render comparison or performance benchmark. Desktop capture remains
affected by the separately reproduced graphics-stack black-frame issue.
