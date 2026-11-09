//! Egui-owned embedded Bevy viewport bridge.
//!
//! This module is intentionally windowless: it gives editor-style
//! apps a Bevy-side scene/viewport state object that can be hosted
//! inside an eframe/Mara shell without Bevy taking ownership of the
//! top-level window. The root `example/` crate uses this as the
//! bridge surface today: native builds render a tiny Bevy scene into
//! an offscreen texture, read the latest rendered frame back, and let
//! the egui host upload that frame as its own texture.

#![cfg_attr(target_arch = "wasm32", allow(dead_code))]
#![allow(clippy::drop_non_drop)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use bevy::app::PluginGroupBuilder;
#[cfg(not(target_arch = "wasm32"))]
use bevy::app::TerminalCtrlCHandlerPlugin;
use bevy::asset::RenderAssetUsages;
use bevy::camera::{ManualTextureViewHandle, RenderTarget};
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::render::{
    Extract, Render, RenderApp, RenderPlugin, RenderSystems,
    render_resource::{
        Buffer, BufferDescriptor, BufferUsages, Extent3d, MapMode, PollType, TexelCopyBufferInfo,
        TexelCopyBufferLayout, Texture, TextureDimension, TextureFormat, TextureUsages,
        TextureView,
    },
    renderer::{
        PendingCommandBuffers, RenderAdapter, RenderAdapterInfo, RenderDevice, RenderGraph,
        RenderGraphSystems, RenderInstance, RenderQueue,
    },
    settings::RenderCreation,
    texture::{ManualTextureView, ManualTextureViews},
};
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use crossbeam_channel::{Receiver, Sender};
use mara_core::vocab::Color32 as MaraColor32;

mod egui_view;
pub use egui_view::MaraBevyViewport;

const EMBEDDED_VIEWPORT_MANUAL_TEXTURE: ManualTextureViewHandle =
    ManualTextureViewHandle(0x4d41_5241);

/// Description of the viewport surface the egui host reserves for
/// Bevy. It is host-facing metadata, not a Bevy window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BevyViewportTexture {
    pub width: u32,
    pub height: u32,
}

impl BevyViewportTexture {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width: width.max(1),
            height: height.max(1),
        }
    }
}

/// CPU-side copy of the latest native Bevy render target.
///
/// This is intentionally plain RGBA bytes so an eframe host can
/// upload it with `egui::Context::load_texture` / `TextureHandle::set`
/// without either renderer taking ownership of the other's swapchain.
#[derive(Debug, Clone)]
pub struct CapturedBevyFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub frame: u64,
}

/// GPU-side Bevy render target for hosts that can sample the
/// viewport texture directly without a CPU readback.
#[derive(Debug, Clone)]
pub struct CapturedBevyTexture {
    pub width: u32,
    pub height: u32,
    pub view: TextureView,
    pub frame: u64,
}

/// Pointer input forwarded by an egui host into the embedded Bevy
/// scene. Coordinates are in physical pixels relative to the viewport
/// image.
#[derive(Debug, Clone, Copy, Default, Resource)]
pub struct BevyViewportInput {
    pub pointer_pos: Option<[f32; 2]>,
    /// Primary-button viewport drag: orbit the camera.
    pub drag_delta: [f32; 2],
    /// Middle-button viewport drag: pan the camera focus.
    pub pan_delta: [f32; 2],
    pub scroll_delta: f32,
    pub primary_clicked: bool,
}

/// Small orbit camera component used by Mara's embedded Bevy demo
/// scene and by Bevy-hosted examples.
///
/// This deliberately lives in `mara_bevy` so the Bevy integration
/// resolves exactly one Bevy version and Mara owns the thin viewport
/// logic it needs.
#[derive(Component, Clone, Copy, Debug)]
pub struct ChaseCamera {
    pub focus: Vec3,
    pub yaw: f32,
    pub elevation: f32,
    pub distance: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    pub min_elevation: f32,
    pub max_elevation: f32,
    pub orbit_speed: f32,
    pub pan_sensitivity: f32,
    pub zoom_step: f64,
}

impl Default for ChaseCamera {
    fn default() -> Self {
        Self {
            focus: Vec3::ZERO,
            yaw: 0.45,
            elevation: 0.35,
            distance: 12.0,
            min_distance: 1.0,
            max_distance: 10_000_000.0,
            min_elevation: -1.45,
            max_elevation: 1.45,
            orbit_speed: 0.005,
            pan_sensitivity: 0.002,
            zoom_step: 0.08,
        }
    }
}

/// Recompute a Bevy transform from a [`ChaseCamera`].
pub fn apply_rig(camera: &ChaseCamera, transform: &mut Transform) {
    let horizontal = camera.elevation.cos();
    let offset = Vec3::new(
        camera.yaw.sin() * horizontal,
        camera.elevation.sin(),
        camera.yaw.cos() * horizontal,
    )
    .normalize_or_zero()
        * camera.distance.max(camera.min_distance);
    transform.translation = camera.focus + offset;
    transform.look_at(camera.focus, Vec3::Y);
}

/// System form of the embedded viewport camera controls. Bevy-owned
/// host apps can run this after they publish [`BevyViewportInput`].
pub fn apply_viewport_camera_input_system(
    input: Res<BevyViewportInput>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform)>,
) {
    for (mut camera, mut transform) in &mut cameras {
        apply_viewport_input_to_camera(*input, &mut camera, &mut transform);
    }
}

fn apply_viewport_input_to_camera(
    input: BevyViewportInput,
    camera: &mut ChaseCamera,
    transform: &mut Transform,
) {
    let drag = Vec2::new(input.drag_delta[0], input.drag_delta[1]);
    let pan = Vec2::new(input.pan_delta[0], input.pan_delta[1]);
    if drag == Vec2::ZERO && pan == Vec2::ZERO && input.scroll_delta == 0.0 {
        return;
    }
    if drag != Vec2::ZERO {
        camera.yaw -= drag.x * camera.orbit_speed;
        camera.elevation += drag.y * camera.orbit_speed;
        camera.elevation = camera
            .elevation
            .clamp(camera.min_elevation, camera.max_elevation);
    }
    if pan != Vec2::ZERO {
        let pan_speed = camera.distance * camera.pan_sensitivity;
        let forward = Vec3::new(camera.yaw.sin(), 0.0, camera.yaw.cos());
        let right = Vec3::new(forward.z, 0.0, -forward.x);
        camera.focus += (-right * pan.x - forward * pan.y) * pan_speed;
    }
    if input.scroll_delta != 0.0 {
        let log_distance = (camera.distance as f64).max(0.1).log10();
        let target = 10f64.powf(log_distance - input.scroll_delta as f64 * camera.zoom_step);
        camera.distance =
            target.clamp(camera.min_distance as f64, camera.max_distance as f64) as f32;
    }
    apply_rig(camera, transform);
}

/// Lightweight X/Z ground grid used by the demo scene.
#[derive(Resource, Clone, Copy, Debug)]
pub struct GroundGrid {
    pub visible: bool,
    pub color: Color,
}

impl Default for GroundGrid {
    fn default() -> Self {
        Self {
            visible: false,
            color: Color::srgba(0.30, 0.38, 0.50, 0.42),
        }
    }
}

/// Draws [`GroundGrid`] with Bevy gizmos when the resource is visible.
pub struct GroundGridPlugin;

#[derive(Resource, Default)]
struct GroundGridPluginInstalled;

impl Plugin for GroundGridPlugin {
    fn build(&self, app: &mut App) {
        if app.world().contains_resource::<GroundGridPluginInstalled>() {
            return;
        }
        app.insert_resource(GroundGridPluginInstalled);
        app.init_resource::<GroundGrid>()
            .add_systems(Update, draw_ground_grid_system);
    }

    fn is_unique(&self) -> bool {
        false
    }
}

/// Small scene-helper bundle for examples and embedded viewports.
///
/// Camera input is opt-in via [`apply_viewport_camera_input_system`]
/// because embedded viewports apply input manually before ticking the
/// offscreen Bevy app.
pub struct MaraBevySceneHelpersPlugin;

impl Plugin for MaraBevySceneHelpersPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(GroundGridPlugin);
    }
}

fn draw_ground_grid_system(
    grid: Option<Res<GroundGrid>>,
    cameras: Query<&ChaseCamera>,
    mut gizmos: Gizmos,
) {
    let Some(grid) = grid else {
        return;
    };
    if !grid.visible {
        return;
    }
    let focus = cameras
        .iter()
        .next()
        .map(|camera| camera.focus)
        .unwrap_or(Vec3::ZERO);
    let step = 1.0;
    let half_lines = 32;
    let half_extent = half_lines as f32 * step;
    let center_x = (focus.x / step).round() * step;
    let center_z = (focus.z / step).round() * step;
    let min_x = center_x - half_extent;
    let max_x = center_x + half_extent;
    let min_z = center_z - half_extent;
    let max_z = center_z + half_extent;
    for i in -half_lines..=half_lines {
        let x = center_x + i as f32 * step;
        let z = center_z + i as f32 * step;
        gizmos.line(
            Vec3::new(x, 0.0, min_z),
            Vec3::new(x, 0.0, max_z),
            grid.color,
        );
        gizmos.line(
            Vec3::new(min_x, 0.0, z),
            Vec3::new(max_x, 0.0, z),
            grid.color,
        );
    }
}

/// Bevy image target created by the viewport infrastructure.
///
/// Example/application content should spawn its camera with
/// `RenderTarget::from(target.0.clone())`.
#[derive(Resource, Clone)]
pub struct BevyViewportRenderTarget(pub Handle<Image>);

/// Optional app/content output used by examples to feed a picked
/// accent colour back to the Mara shell.
#[derive(Resource, Default, Clone, Copy)]
pub struct BevyViewportPickedColor(pub Option<MaraColor32>);

/// Startup set that creates [`BevyViewportRenderTarget`].
///
/// Content setup systems that need the render target should run
/// `.after(BevyViewportSet::SetupTarget)`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BevyViewportSet {
    SetupTarget,
}

/// Hook used by consumers to add scene/content systems to the
/// windowless Bevy app owned by the viewport renderer.
pub type BevyViewportAppConfigure = Arc<dyn Fn(&mut App) + Send + Sync + 'static>;

/// Hook that adjusts the embedded app's `DefaultPlugins` before they are
/// added, for settings that only take effect at plugin build time (an
/// `AssetPlugin` root, for one).
pub type BevyViewportPluginsConfigure =
    Arc<dyn Fn(PluginGroupBuilder) -> PluginGroupBuilder + Send + Sync + 'static>;

/// Existing wgpu resources supplied by an egui/eframe host.
///
/// Passing these keeps the embedded Bevy renderer on the same adapter
/// that already successfully opened the eframe window instead of
/// asking wgpu for a second headless adapter.
#[derive(Clone)]
pub struct BevyViewportWgpuResources {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: wgpu::Adapter,
}

impl BevyViewportWgpuResources {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, adapter: wgpu::Adapter) -> Self {
        Self {
            device,
            queue,
            adapter,
        }
    }
}

/// Windowless Bevy scene state for an embedded viewport.
///
/// This keeps a real Bevy [`World`] and updates Bevy transforms every
/// frame, but does not install `WinitPlugin` or create a Bevy window.
/// It is deliberately small so consumers can embed it in any egui host.
pub struct BevyViewportBridge {
    frame: u64,
    seconds: f32,
    texture: BevyViewportTexture,
    external_wgpu: Option<BevyViewportWgpuResources>,
    renderer: Option<BevyViewportRenderer>,
    renderer_failed: bool,
    rendering_enabled: bool,
    latest_frame: Option<CapturedBevyFrame>,
    input: BevyViewportInput,
    scene_state: Option<EmbeddedViewportSceneState>,
    configure_plugins: Option<BevyViewportPluginsConfigure>,
    configure_app: Option<BevyViewportAppConfigure>,
}

impl Default for BevyViewportBridge {
    fn default() -> Self {
        Self::new(BevyViewportTexture::new(512, 512))
    }
}

impl BevyViewportBridge {
    /// The embedded app's world, `None` until the renderer is created.
    pub fn world_mut(&mut self) -> Option<&mut World> {
        self.renderer.as_mut().map(BevyViewportRenderer::world_mut)
    }

    pub fn new(texture: BevyViewportTexture) -> Self {
        Self {
            frame: 0,
            seconds: 0.0,
            texture,
            external_wgpu: None,
            renderer: None,
            renderer_failed: false,
            rendering_enabled: true,
            latest_frame: None,
            input: BevyViewportInput::default(),
            scene_state: None,
            configure_plugins: None,
            configure_app: None,
        }
    }

    /// Install a hook that adjusts `DefaultPlugins` before the embedded app
    /// is built. Must be set before the first frame creates the renderer.
    pub fn set_plugins_config(
        &mut self,
        configure_plugins: impl Fn(PluginGroupBuilder) -> PluginGroupBuilder + Send + Sync + 'static,
    ) {
        self.configure_plugins = Some(Arc::new(configure_plugins));
    }

    pub fn with_app_config(
        texture: BevyViewportTexture,
        configure_app: impl Fn(&mut App) + Send + Sync + 'static,
    ) -> Self {
        let mut bridge = Self::new(texture);
        bridge.configure_app = Some(Arc::new(configure_app));
        bridge
    }

    pub fn with_wgpu_resources(
        texture: BevyViewportTexture,
        resources: BevyViewportWgpuResources,
    ) -> Self {
        let mut bridge = Self::new(texture);
        bridge.external_wgpu = Some(resources);
        bridge
    }

    pub fn with_wgpu_resources_and_app_config(
        texture: BevyViewportTexture,
        resources: BevyViewportWgpuResources,
        configure_app: impl Fn(&mut App) + Send + Sync + 'static,
    ) -> Self {
        let mut bridge = Self::with_app_config(texture, configure_app);
        bridge.external_wgpu = Some(resources);
        bridge
    }

    pub fn attach_wgpu_resources(&mut self, resources: BevyViewportWgpuResources) {
        if self.renderer.is_none() && !self.renderer_failed {
            self.external_wgpu = Some(resources);
        }
    }

    /// Enable or disable the embedded Bevy render loop.
    ///
    /// Hosts should call this when the viewport's Mara view is not
    /// the active content surface. Disabling keeps the latest
    /// captured frame/texture around, but prevents `app.update()`
    /// and flips Bevy cameras inactive so no offscreen camera work is
    /// scheduled while another Mara view is foregrounded.
    pub fn set_rendering_enabled(&mut self, enabled: bool) {
        if self.rendering_enabled == enabled {
            return;
        }
        self.rendering_enabled = enabled;
        self.input = BevyViewportInput::default();
        if let Some(renderer) = &mut self.renderer {
            renderer.set_cameras_active(enabled);
        }
    }

    #[must_use]
    pub fn rendering_enabled(&self) -> bool {
        self.rendering_enabled
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.texture = BevyViewportTexture::new(width, height);
    }

    pub fn tick(&mut self, dt_seconds: f32) {
        self.frame = self.frame.saturating_add(1);
        self.seconds += dt_seconds.max(0.0);
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn seconds(&self) -> f32 {
        self.seconds
    }

    pub fn texture(&self) -> BevyViewportTexture {
        self.texture
    }

    pub fn rotation_angle(&self) -> f32 {
        self.seconds * 0.75
    }

    /// Allocate a Bevy `Image` suitable for an offscreen camera render
    /// target with this bridge's current viewport size.
    pub fn render_target_image(&self, format: TextureFormat) -> Image {
        make_viewport_render_target(self.texture, format)
    }

    /// Render one native Bevy frame for the current viewport size and
    /// return the newest captured RGBA frame if the render pipeline has
    /// produced one.
    ///
    /// This is the safe bridge path from the plan: Bevy owns a
    /// windowless offscreen render target, while egui/eframe remains
    /// the top-level window and uploads the returned pixels as an egui
    /// texture. A future zero-copy path can replace the CPU readback
    /// without changing the app-shell ownership model.
    pub fn render_frame(
        &mut self,
        width: u32,
        height: u32,
        dt_seconds: f32,
    ) -> Option<&CapturedBevyFrame> {
        self.render_frame_with_input(width, height, dt_seconds, BevyViewportInput::default())
    }

    pub fn render_frame_with_input(
        &mut self,
        width: u32,
        height: u32,
        dt_seconds: f32,
        input: BevyViewportInput,
    ) -> Option<&CapturedBevyFrame> {
        if !self.rendering_enabled {
            self.input = BevyViewportInput::default();
            return self.latest_frame.as_ref();
        }

        self.resize(width, height);
        self.tick(dt_seconds);
        self.input = input;

        if self.renderer_failed {
            return self.latest_frame.as_ref();
        }

        let texture = self.texture;
        let rendered = catch_unwind(AssertUnwindSafe(|| {
            let renderer = self.renderer.get_or_insert_with(|| {
                BevyViewportRenderer::new(
                    texture,
                    self.external_wgpu.clone(),
                    self.configure_plugins.clone(),
                    self.configure_app.clone(),
                )
            });
            renderer.set_cameras_active(true);
            if renderer.texture() != texture {
                let scene_state = renderer.scene_state().or(self.scene_state);
                if !renderer.resize(texture) {
                    *renderer = BevyViewportRenderer::new(
                        texture,
                        self.external_wgpu.clone(),
                        self.configure_plugins.clone(),
                        self.configure_app.clone(),
                    );
                    if let Some(scene_state) = scene_state {
                        renderer.apply_scene_state(scene_state);
                    }
                }
            }
            renderer.set_input(self.input);
            let frame = renderer.render_next();
            self.scene_state = renderer.scene_state();
            frame
        }));

        match rendered {
            Ok(Some(mut frame)) => {
                frame.frame = self.frame;
                self.latest_frame = Some(frame);
            }
            Ok(None) => {}
            Err(_) => {
                self.renderer = None;
                self.renderer_failed = true;
            }
        }
        self.latest_frame.as_ref()
    }

    pub fn render_texture_with_input(
        &mut self,
        width: u32,
        height: u32,
        dt_seconds: f32,
        input: BevyViewportInput,
    ) -> Option<CapturedBevyTexture> {
        if !self.rendering_enabled {
            self.input = BevyViewportInput::default();
            return None;
        }

        self.resize(width, height);
        self.tick(dt_seconds);
        self.input = input;

        if self.renderer_failed {
            return None;
        }

        let texture = self.texture;
        let rendered = catch_unwind(AssertUnwindSafe(|| {
            let renderer = self.renderer.get_or_insert_with(|| {
                BevyViewportRenderer::new(
                    texture,
                    self.external_wgpu.clone(),
                    self.configure_plugins.clone(),
                    self.configure_app.clone(),
                )
            });
            renderer.set_cameras_active(true);
            if renderer.texture() != texture {
                let scene_state = renderer.scene_state().or(self.scene_state);
                if !renderer.resize(texture) {
                    *renderer = BevyViewportRenderer::new(
                        texture,
                        self.external_wgpu.clone(),
                        self.configure_plugins.clone(),
                        self.configure_app.clone(),
                    );
                    if let Some(scene_state) = scene_state {
                        renderer.apply_scene_state(scene_state);
                    }
                }
            }
            renderer.set_input(self.input);
            let mut frame = renderer.render_next_texture();
            self.scene_state = renderer.scene_state();
            if let Some(frame) = &mut frame {
                frame.frame = self.frame;
            }
            frame
        }));

        match rendered {
            Ok(frame) => frame,
            Err(_) => {
                self.renderer = None;
                self.renderer_failed = true;
                None
            }
        }
    }

    /// Returns true after Bevy failed to initialize or tick its
    /// native renderer, usually because the process has no usable
    /// `wgpu` adapter. The egui host can keep the app shell alive and
    /// display a fallback instead of crashing.
    pub fn renderer_failed(&self) -> bool {
        self.renderer_failed
    }

    /// Optional color emitted by the consumer's Bevy content during
    /// the last rendered frame.
    ///
    /// Content can set [`BevyViewportPickedColor`] to feed an accent
    /// or selection color back to the Mara/egui host.
    pub fn picked_color(&self) -> Option<MaraColor32> {
        self.scene_state.and_then(|state| state.picked_color)
    }
}

/// Allocate a Bevy-owned render-target image for an embedded
/// viewport. The usage flags are the important contract:
/// `RENDER_ATTACHMENT` lets a Bevy camera render into it,
/// `TEXTURE_BINDING` lets a UI/material sample it, and `COPY_SRC`
/// leaves room for host bridges that need to copy into another
/// renderer's texture.
pub fn make_viewport_render_target(texture: BevyViewportTexture, format: TextureFormat) -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: texture.width,
            height: texture.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        format,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC;
    image
}

fn make_embedded_viewport_render_target(width: u32, height: u32) -> Image {
    // Compatibility handle for existing content that expects a
    // BevyViewportRenderTarget image. Embedded cameras are redirected
    // to a manual GPU texture view after content setup, so the fast
    // path does not render into this image or copy from it.
    let mut image = Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC | TextureUsages::TEXTURE_BINDING;
    image
}

/// Spawn a Bevy 3D camera configured to render into a viewport image
/// instead of a window. Consumers add the returned entity to their
/// own windowless Bevy app/world and store `target` in `Assets<Image>`.
pub fn spawn_viewport_camera(world: &mut World, target: Handle<Image>) -> Entity {
    world
        .spawn((
            Camera3d::default(),
            Camera::default(),
            RenderTarget::from(target),
            Transform::from_xyz(0.0, 2.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
            GlobalTransform::default(),
        ))
        .id()
}

/// Native headless Bevy renderer used by [`BevyViewportBridge`].
///
/// The renderer owns a small Bevy [`App`] with no primary window and
/// no `WinitPlugin`. The scene camera targets a Bevy `Image`; the
/// render graph copies that image into a CPU buffer and sends compact
/// RGBA frames back to the main world.
pub struct BevyViewportRenderer {
    app: App,
    texture: BevyViewportTexture,
    input: BevyViewportInput,
    resize_warmup_frames: u8,
}

#[derive(Clone, Copy, Debug)]
struct EmbeddedViewportSceneState {
    focus: Vec3,
    yaw: f32,
    elevation: f32,
    distance: f32,
    picked_color: Option<MaraColor32>,
}

impl BevyViewportRenderer {
    pub fn world_mut(&mut self) -> &mut World {
        self.app.world_mut()
    }

    pub fn new(
        texture: BevyViewportTexture,
        resources: Option<BevyViewportWgpuResources>,
        configure_plugins: Option<BevyViewportPluginsConfigure>,
        configure_app: Option<BevyViewportAppConfigure>,
    ) -> Self {
        let mut app = App::new();
        let mut default_plugins = DefaultPlugins
            .set(ImagePlugin::default_nearest())
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .disable::<WinitPlugin>();
        if let Some(resources) = resources {
            let adapter_info = resources.adapter.get_info();
            let instance = wgpu::Instance::default();
            default_plugins = default_plugins.set(RenderPlugin {
                render_creation: RenderCreation::manual(
                    RenderDevice::from(resources.device),
                    RenderQueue::new(resources.queue),
                    RenderAdapterInfo::new(adapter_info),
                    RenderAdapter::new(resources.adapter),
                    RenderInstance::new(instance),
                ),
                ..default()
            });
        }
        let default_plugins = default_plugins.disable::<LogPlugin>();
        #[cfg(not(target_arch = "wasm32"))]
        let default_plugins = default_plugins.disable::<TerminalCtrlCHandlerPlugin>();
        let default_plugins = match configure_plugins {
            Some(configure) => configure(default_plugins),
            None => default_plugins,
        };

        app.insert_resource(EmbeddedViewportConfig { texture })
            .init_resource::<BevyViewportPickedColor>()
            .add_plugins(default_plugins)
            // The embedded viewport receives pointer/scroll input
            // from egui and applies it manually before each Bevy
            // tick, so the helper plugin only contributes visual
            // scene helpers such as the optional ground grid.
            .add_plugins(MaraBevySceneHelpersPlugin)
            .configure_sets(Startup, BevyViewportSet::SetupTarget)
            .add_systems(
                Startup,
                setup_embedded_viewport_target.in_set(BevyViewportSet::SetupTarget),
            )
            .add_systems(Update, redirect_embedded_viewport_cameras_to_manual_texture);
        app.add_plugins(EmbeddedViewportCopyPlugin);

        if let Some(configure_app) = configure_app {
            configure_app(&mut app);
        }

        // We drive this embedded app manually with `app.update()`
        // instead of `app.run()`, so Bevy's plugin lifecycle must be
        // completed explicitly. Renderer/PBR resources such as
        // `MeshPipeline` are created in plugin finish/cleanup hooks.
        app.finish();
        app.cleanup();

        Self {
            app,
            texture,
            input: BevyViewportInput::default(),
            resize_warmup_frames: 0,
        }
    }

    pub fn texture(&self) -> BevyViewportTexture {
        self.texture
    }

    pub fn set_cameras_active(&mut self, active: bool) {
        let world = self.app.world_mut();
        let mut cameras = world.query::<&mut Camera>();
        for mut camera in cameras.iter_mut(world) {
            camera.is_active = active;
        }
    }

    pub fn resize(&mut self, texture: BevyViewportTexture) -> bool {
        if self.texture == texture {
            return true;
        }

        let world = self.app.world_mut();
        let Some(render_target) = world.get_resource::<BevyViewportRenderTarget>().cloned() else {
            return false;
        };
        let src_image = render_target.0;

        let render_target_image =
            make_embedded_viewport_render_target(texture.width, texture.height);

        let Some(mut images) = world.get_resource_mut::<Assets<Image>>() else {
            return false;
        };
        if images.insert(src_image.id(), render_target_image).is_err() {
            return false;
        }

        if let Some(render_device) = world.get_resource::<RenderDevice>().cloned() {
            let size = Extent3d {
                width: texture.width,
                height: texture.height,
                depth_or_array_layers: 1,
            };
            let replacement = EmbeddedViewportImageCopier::new(size, render_device.wgpu_device());
            if let Some(mut manual_views) = world.get_resource_mut::<ManualTextureViews>() {
                manual_views.insert(
                    EMBEDDED_VIEWPORT_MANUAL_TEXTURE,
                    replacement.manual_texture_view(),
                );
            }
            let mut copiers = world.query::<&mut EmbeddedViewportImageCopier>();
            for mut copier in copiers.iter_mut(world) {
                *copier = replacement.clone();
            }

            if let Some(receiver) = world.get_resource::<EmbeddedViewportReceiver>() {
                while receiver.try_recv().is_ok() {}
            }
        }

        self.texture = texture;
        self.resize_warmup_frames = 0;
        true
    }

    pub fn set_input(&mut self, input: BevyViewportInput) {
        self.input = input;
    }

    pub fn render_next(&mut self) -> Option<CapturedBevyFrame> {
        self.set_cpu_readback_enabled(true);
        self.app.world_mut().insert_resource(self.input);
        apply_embedded_viewport_input(self.app.world_mut(), self.input);
        self.app.update();

        let receiver = self.app.world().resource::<EmbeddedViewportReceiver>();
        let mut latest = None;
        while let Ok(frame) = receiver.try_recv() {
            if frame.width != self.texture.width || frame.height != self.texture.height {
                continue;
            }
            if frame.rgba.iter().all(|&byte| byte == 0) {
                continue;
            }
            if self.resize_warmup_frames > 0 {
                self.resize_warmup_frames -= 1;
                continue;
            }
            latest = Some(frame);
        }
        latest
    }

    pub fn render_next_texture(&mut self) -> Option<CapturedBevyTexture> {
        self.set_cpu_readback_enabled(false);
        self.app.world_mut().insert_resource(self.input);
        apply_embedded_viewport_input(self.app.world_mut(), self.input);
        self.app.update();
        self.gpu_texture()
    }

    fn set_cpu_readback_enabled(&mut self, enabled: bool) {
        let world = self.app.world_mut();
        let mut copiers = world.query::<&EmbeddedViewportImageCopier>();
        for copier in copiers.iter(world) {
            copier.set_enabled(enabled);
        }
    }

    fn gpu_texture(&mut self) -> Option<CapturedBevyTexture> {
        let world = self.app.world_mut();
        let mut copiers = world.query::<&EmbeddedViewportImageCopier>();
        let copier = copiers.iter(world).next()?;
        let copied_frame = copier.copied_frame.load(Ordering::Acquire);
        // Do not hand a freshly allocated egui texture to the host
        // until the render graph has copied into it across at least
        // one completed update. This keeps resize from flashing an
        // empty/black texture while the new target warms up.
        if copied_frame < 2 {
            return None;
        }
        Some(CapturedBevyTexture {
            width: copier.size.width,
            height: copier.size.height,
            view: copier.egui_texture_view.clone(),
            frame: copied_frame,
        })
    }

    fn scene_state(&mut self) -> Option<EmbeddedViewportSceneState> {
        let world = self.app.world_mut();
        let mut cameras = world.query::<&ChaseCamera>();
        let camera = cameras.iter(world).next()?;
        let picked_color = world
            .get_resource::<BevyViewportPickedColor>()
            .and_then(|selected| selected.0);
        Some(EmbeddedViewportSceneState {
            focus: camera.focus,
            yaw: camera.yaw,
            elevation: camera.elevation,
            distance: camera.distance,
            picked_color,
        })
    }

    fn apply_scene_state(&mut self, scene_state: EmbeddedViewportSceneState) {
        let world = self.app.world_mut();
        {
            let mut cameras = world.query::<(&mut ChaseCamera, &mut Transform)>();
            for (mut camera, mut transform) in cameras.iter_mut(world) {
                camera.focus = scene_state.focus;
                camera.yaw = scene_state.yaw;
                camera.elevation = scene_state.elevation;
                camera.distance = scene_state.distance;
                apply_rig(&camera, &mut transform);
            }
        }
    }
}

fn apply_embedded_viewport_input(world: &mut World, input: BevyViewportInput) {
    let mut cameras = world.query::<(&mut ChaseCamera, &mut Transform)>();
    for (mut camera, mut transform) in cameras.iter_mut(world) {
        apply_viewport_input_to_camera(input, &mut camera, &mut transform);
    }
}

#[derive(Resource)]
struct EmbeddedViewportConfig {
    texture: BevyViewportTexture,
}

#[cfg(not(target_arch = "wasm32"))]
fn setup_embedded_viewport_target(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    render_device: Res<RenderDevice>,
    config: Res<EmbeddedViewportConfig>,
) {
    let size = Extent3d {
        width: config.texture.width,
        height: config.texture.height,
        depth_or_array_layers: 1,
    };

    let render_target_image = make_embedded_viewport_render_target(size.width, size.height);
    let render_target_handle = images.add(render_target_image);

    let image_copier = EmbeddedViewportImageCopier::new(size, render_device.wgpu_device());
    let manual_view = image_copier.manual_texture_view();
    commands.spawn(image_copier);
    commands.queue(move |world: &mut World| {
        world
            .resource_mut::<ManualTextureViews>()
            .insert(EMBEDDED_VIEWPORT_MANUAL_TEXTURE, manual_view);
    });
    commands.insert_resource(BevyViewportRenderTarget(render_target_handle));
}

#[cfg(target_arch = "wasm32")]
fn setup_embedded_viewport_target(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    render_device: Option<Res<RenderDevice>>,
    config: Res<EmbeddedViewportConfig>,
) {
    let size = Extent3d {
        width: config.texture.width,
        height: config.texture.height,
        depth_or_array_layers: 1,
    };

    let render_target_image = make_embedded_viewport_render_target(size.width, size.height);
    let render_target_handle = images.add(render_target_image);

    if let Some(render_device) = render_device {
        let image_copier = EmbeddedViewportImageCopier::new(size, render_device.wgpu_device());
        let manual_view = image_copier.manual_texture_view();
        commands.spawn(image_copier);
        commands.queue(move |world: &mut World| {
            world
                .resource_mut::<ManualTextureViews>()
                .insert(EMBEDDED_VIEWPORT_MANUAL_TEXTURE, manual_view);
        });
    }
    commands.insert_resource(BevyViewportRenderTarget(render_target_handle));
}

fn redirect_embedded_viewport_cameras_to_manual_texture(
    render_target: Option<Res<BevyViewportRenderTarget>>,
    mut cameras: Query<&mut RenderTarget>,
) {
    let Some(render_target) = render_target else {
        return;
    };
    for mut target in &mut cameras {
        if target.as_image() == Some(&render_target.0) {
            *target = RenderTarget::TextureView(EMBEDDED_VIEWPORT_MANUAL_TEXTURE);
        }
    }
}

#[derive(Resource)]
struct EmbeddedViewportReceiver(Receiver<CapturedBevyFrame>);

impl std::ops::Deref for EmbeddedViewportReceiver {
    type Target = Receiver<CapturedBevyFrame>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Resource)]
struct EmbeddedViewportSender(Sender<CapturedBevyFrame>);

impl std::ops::Deref for EmbeddedViewportSender {
    type Target = Sender<CapturedBevyFrame>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

struct MappedBevyFrame {
    buffer: Buffer,
    pending_map: Arc<AtomicBool>,
    width: u32,
    height: u32,
    row_bytes: usize,
    padded_row_bytes: usize,
}

#[derive(Resource)]
struct EmbeddedViewportMappedReceiver(Receiver<MappedBevyFrame>);

impl std::ops::Deref for EmbeddedViewportMappedReceiver {
    type Target = Receiver<MappedBevyFrame>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Resource)]
struct EmbeddedViewportMappedSender(Sender<MappedBevyFrame>);

impl std::ops::Deref for EmbeddedViewportMappedSender {
    type Target = Sender<MappedBevyFrame>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

struct EmbeddedViewportCopyPlugin;

impl Plugin for EmbeddedViewportCopyPlugin {
    fn build(&self, app: &mut App) {
        let (sender, receiver) = crossbeam_channel::unbounded();
        app.insert_resource(EmbeddedViewportReceiver(receiver));

        let render_app = app.sub_app_mut(RenderApp);
        render_app.insert_resource(EmbeddedViewportSender(sender));
        let (mapped_sender, mapped_receiver) = crossbeam_channel::unbounded();
        render_app.insert_resource(EmbeddedViewportMappedSender(mapped_sender));
        render_app.insert_resource(EmbeddedViewportMappedReceiver(mapped_receiver));

        render_app
            .add_systems(ExtractSchedule, extract_embedded_viewport_copiers)
            .add_systems(
                Render,
                receive_embedded_viewport_frames.after(RenderSystems::Render),
            )
            .add_systems(
                RenderGraph,
                copy_embedded_viewport_textures_system
                    .after(RenderGraphSystems::Render)
                    .before(RenderGraphSystems::Submit),
            );
    }
}

#[derive(Clone, Default, Resource)]
struct EmbeddedViewportCopiers(Vec<EmbeddedViewportImageCopier>);

#[derive(Clone, Component)]
struct EmbeddedViewportImageCopier {
    buffer: Buffer,
    size: Extent3d,
    egui_texture: Texture,
    egui_texture_view: TextureView,
    bevy_texture_view: TextureView,
    copied_frame: Arc<AtomicU64>,
    enabled: Arc<AtomicBool>,
    pending_map: Arc<AtomicBool>,
}

impl EmbeddedViewportImageCopier {
    fn new(size: Extent3d, device: &wgpu::Device) -> Self {
        let row_bytes = size.width as usize * 4;
        let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(row_bytes);
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("mara_embedded_bevy_viewport_readback"),
            size: padded_bytes_per_row as u64 * size.height as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let egui_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mara_embedded_bevy_viewport_egui_texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            ],
        });
        let egui_texture_view = egui_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bevy_texture_view = egui_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("mara_embedded_bevy_viewport_srgb_render_view"),
            format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            ..Default::default()
        });

        Self {
            buffer: buffer.into(),
            size,
            egui_texture: egui_texture.into(),
            egui_texture_view: egui_texture_view.into(),
            bevy_texture_view: bevy_texture_view.into(),
            copied_frame: Arc::new(AtomicU64::new(0)),
            enabled: Arc::new(AtomicBool::new(true)),
            pending_map: Arc::new(AtomicBool::new(false)),
        }
    }

    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    fn ready_for_copy(&self) -> bool {
        self.enabled() && !self.pending_map.load(Ordering::Relaxed)
    }

    fn mark_pending_map(&self) -> bool {
        self.pending_map
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn manual_texture_view(&self) -> ManualTextureView {
        ManualTextureView {
            texture_view: self.bevy_texture_view.clone(),
            size: UVec2::new(self.size.width, self.size.height),
            view_format: TextureFormat::Rgba8UnormSrgb,
        }
    }
}

fn extract_embedded_viewport_copiers(
    mut commands: Commands,
    image_copiers: Extract<Query<&EmbeddedViewportImageCopier>>,
) {
    commands.insert_resource(EmbeddedViewportCopiers(
        image_copiers.iter().cloned().collect(),
    ));
}

fn copy_embedded_viewport_textures_system(
    image_copiers: Res<EmbeddedViewportCopiers>,
    mapped_sender: Res<EmbeddedViewportMappedSender>,
    render_device: Res<RenderDevice>,
    mut pending_buffers: ResMut<PendingCommandBuffers>,
) {
    let mut encoder =
        render_device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let mut any = false;

    for image_copier in &image_copiers.0 {
        image_copier.copied_frame.fetch_add(1, Ordering::Release);

        if image_copier.ready_for_copy() && image_copier.mark_pending_map() {
            let width = image_copier.size.width;
            let height = image_copier.size.height;
            let row_bytes = width as usize * 4;
            let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(row_bytes);

            encoder.copy_texture_to_buffer(
                image_copier.egui_texture.as_image_copy(),
                TexelCopyBufferInfo {
                    buffer: &image_copier.buffer,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(
                            std::num::NonZero::<u32>::new(padded_bytes_per_row as u32)
                                .expect("non-zero row bytes")
                                .into(),
                        ),
                        rows_per_image: None,
                    },
                },
                image_copier.size,
            );

            let buffer_for_callback = image_copier.buffer.clone();
            let pending_map = image_copier.pending_map.clone();
            let mapped_sender = mapped_sender.0.clone();
            encoder.map_buffer_on_submit(&image_copier.buffer, MapMode::Read, .., move |result| {
                if result.is_ok() {
                    let mapped_frame = MappedBevyFrame {
                        buffer: buffer_for_callback.clone(),
                        pending_map: pending_map.clone(),
                        width,
                        height,
                        row_bytes,
                        padded_row_bytes: padded_bytes_per_row,
                    };
                    if mapped_sender.send(mapped_frame).is_err() {
                        buffer_for_callback.unmap();
                        pending_map.store(false, Ordering::Release);
                    }
                } else {
                    pending_map.store(false, Ordering::Release);
                }
            });
            any = true;
        }
    }

    if any {
        pending_buffers.push_encoder(encoder, "embedded_viewport_readback");
    }
}
fn receive_embedded_viewport_frames(
    render_device: Res<RenderDevice>,
    sender: Res<EmbeddedViewportSender>,
    mapped_receiver: Res<EmbeddedViewportMappedReceiver>,
) {
    let _ = render_device.poll(PollType::Poll);

    while let Ok(mapped_frame) = mapped_receiver.try_recv() {
        let buffer_slice = mapped_frame.buffer.slice(..);
        // The map callback only fires after a successful `map_async`, so the
        // range is always available here.
        let mapped = buffer_slice
            .get_mapped_range()
            .expect("embedded viewport readback buffer is mapped");
        let rgba = if mapped_frame.row_bytes == mapped_frame.padded_row_bytes {
            mapped[..mapped_frame.row_bytes * mapped_frame.height as usize].to_vec()
        } else {
            mapped
                .chunks(mapped_frame.padded_row_bytes)
                .take(mapped_frame.height as usize)
                .flat_map(|row| row[..mapped_frame.row_bytes.min(row.len())].iter().copied())
                .collect()
        };
        drop(mapped);
        mapped_frame.buffer.unmap();
        mapped_frame.pending_map.store(false, Ordering::Release);

        let _ = sender.send(CapturedBevyFrame {
            width: mapped_frame.width,
            height: mapped_frame.height,
            rgba,
            frame: 0,
        });
    }
}

/// Top-level embedded Bevy view model for egui-hosted apps.
#[derive(Default)]
pub struct BevyEmbeddedView {
    bridge: BevyViewportBridge,
}

impl BevyEmbeddedView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn world_mut(&mut self) -> Option<&mut World> {
        self.bridge.world_mut()
    }

    pub fn set_plugins_config(
        &mut self,
        configure_plugins: impl Fn(PluginGroupBuilder) -> PluginGroupBuilder + Send + Sync + 'static,
    ) {
        self.bridge.set_plugins_config(configure_plugins);
    }

    pub fn with_app_config(configure_app: impl Fn(&mut App) + Send + Sync + 'static) -> Self {
        Self {
            bridge: BevyViewportBridge::with_app_config(
                BevyViewportTexture::new(512, 512),
                configure_app,
            ),
        }
    }

    pub fn with_wgpu_resources(resources: BevyViewportWgpuResources) -> Self {
        Self {
            bridge: BevyViewportBridge::with_wgpu_resources(
                BevyViewportTexture::new(512, 512),
                resources,
            ),
        }
    }

    pub fn with_wgpu_resources_and_app_config(
        resources: BevyViewportWgpuResources,
        configure_app: impl Fn(&mut App) + Send + Sync + 'static,
    ) -> Self {
        Self {
            bridge: BevyViewportBridge::with_wgpu_resources_and_app_config(
                BevyViewportTexture::new(512, 512),
                resources,
                configure_app,
            ),
        }
    }

    pub fn attach_wgpu_resources(&mut self, resources: BevyViewportWgpuResources) {
        self.bridge.attach_wgpu_resources(resources);
    }

    pub fn set_rendering_enabled(&mut self, enabled: bool) {
        self.bridge.set_rendering_enabled(enabled);
    }

    #[must_use]
    pub fn rendering_enabled(&self) -> bool {
        self.bridge.rendering_enabled()
    }

    pub fn bridge(&self) -> &BevyViewportBridge {
        &self.bridge
    }

    pub fn bridge_mut(&mut self) -> &mut BevyViewportBridge {
        &mut self.bridge
    }

    pub fn render_frame(
        &mut self,
        width: u32,
        height: u32,
        dt_seconds: f32,
    ) -> Option<&CapturedBevyFrame> {
        self.bridge.render_frame(width, height, dt_seconds)
    }

    pub fn render_frame_with_input(
        &mut self,
        width: u32,
        height: u32,
        dt_seconds: f32,
        input: BevyViewportInput,
    ) -> Option<&CapturedBevyFrame> {
        self.bridge
            .render_frame_with_input(width, height, dt_seconds, input)
    }

    pub fn render_texture_with_input(
        &mut self,
        width: u32,
        height: u32,
        dt_seconds: f32,
        input: BevyViewportInput,
    ) -> Option<CapturedBevyTexture> {
        self.bridge
            .render_texture_with_input(width, height, dt_seconds, input)
    }

    pub fn renderer_failed(&self) -> bool {
        self.bridge.renderer_failed()
    }

    pub fn picked_color(&self) -> Option<MaraColor32> {
        self.bridge.picked_color()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_view_ticks_bevy_world_without_window() {
        let mut view = BevyEmbeddedView::new();
        assert_eq!(view.bridge().frame(), 0);
        view.bridge_mut().tick(1.0 / 60.0);
        assert_eq!(view.bridge().frame(), 1);
        assert!(view.bridge().rotation_angle().is_finite());
    }

    #[test]
    fn viewport_texture_clamps_to_non_zero_size() {
        let texture = BevyViewportTexture::new(0, 0);
        assert_eq!(texture.width, 1);
        assert_eq!(texture.height, 1);
    }

    #[test]
    fn render_target_image_has_camera_and_sampling_usage() {
        let bridge = BevyViewportBridge::new(BevyViewportTexture::new(64, 32));
        let image = bridge.render_target_image(TextureFormat::Bgra8UnormSrgb);
        assert_eq!(image.texture_descriptor.size.width, 64);
        assert_eq!(image.texture_descriptor.size.height, 32);
        assert!(
            image
                .texture_descriptor
                .usage
                .contains(TextureUsages::RENDER_ATTACHMENT)
        );
        assert!(
            image
                .texture_descriptor
                .usage
                .contains(TextureUsages::TEXTURE_BINDING)
        );
    }

    #[test]
    fn viewport_camera_targets_image_not_window() {
        let mut world = World::new();
        let handle = Handle::<Image>::default();
        let camera = spawn_viewport_camera(&mut world, handle.clone());
        let target = world
            .get::<RenderTarget>(camera)
            .expect("render target component");
        assert_eq!(target.as_image(), Some(&handle));
    }

    #[test]
    fn ground_grid_plugin_is_idempotent() {
        let mut app = App::new();
        app.add_plugins(GroundGridPlugin);
        app.add_plugins(GroundGridPlugin);
    }

    #[test]
    #[ignore = "requires a native wgpu adapter; run manually when validating the embedded viewport"]
    fn headless_renderer_produces_rgba_frame() {
        let mut bridge = BevyViewportBridge::new(BevyViewportTexture::new(96, 64));
        let mut captured = None;
        for _ in 0..120 {
            if let Some(frame) = bridge.render_frame(96, 64, 1.0 / 60.0)
                && (frame.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0)
                    || frame.rgba.iter().any(|&byte| byte != 0))
            {
                captured = Some(frame.clone());
                break;
            }
            if bridge.renderer_failed() {
                eprintln!("skipping embedded renderer smoke test: no native wgpu adapter");
                return;
            }
        }

        let frame = captured.expect("headless renderer produced a frame");
        assert_eq!(frame.width, 96);
        assert_eq!(frame.height, 64);
        assert_eq!(frame.rgba.len(), 96 * 64 * 4);
        assert!(frame.rgba.iter().any(|&byte| byte != 0));
    }

    #[test]
    #[ignore = "requires a native wgpu adapter; run manually when validating the embedded viewport"]
    fn headless_renderer_produces_gpu_texture() {
        let mut bridge = BevyViewportBridge::new(BevyViewportTexture::new(96, 64));
        let mut captured = None;
        for _ in 0..120 {
            if let Some(texture) =
                bridge.render_texture_with_input(96, 64, 1.0 / 60.0, BevyViewportInput::default())
            {
                captured = Some(texture);
                break;
            }
            if bridge.renderer_failed() {
                eprintln!("skipping embedded renderer smoke test: no native wgpu adapter");
                return;
            }
        }

        let texture = captured.expect("headless renderer produced a GPU texture");
        assert_eq!(texture.width, 96);
        assert_eq!(texture.height, 64);
    }
}
