# Bevy PBR 0.19.1

Copied from the workspace's installed crates.io Bevy PBR 0.19.1 source.
Rendering code is unchanged. The normalized manifest's
`pbr_multi_layer_material_textures` feature omits its weak optional glTF
forwarding edge, which otherwise requires unavailable `bevy_gltf` 0.19.1
metadata during dependency resolution. This USD viewer does not enable glTF.
The direct usd_bevy dependency enables PBR texture support without forwarding
through Bevy's optional glTF integration. Remove this manifest patch when the
matching upstream glTF dependency is available.
