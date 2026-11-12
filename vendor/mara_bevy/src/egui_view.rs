//! Mara/egui-hosted Bevy viewport widget.
//!
//! This is the host-facing view wrapper around [`BevyEmbeddedView`]:
//! it owns the egui allocation, interaction forwarding, resize
//! throttling, texture upload, and warmup/fallback painting. Example
//! apps should only hold this state and place it in their content
//! tree; the viewport mechanics live here in the module crate.

use std::time::Duration;

use mara_backend_egui::EguiCtx;
use mara_core::MaraPainter;
use mara_core::context::MaraCtx;
use mara_core::layout::Layer;
use mara_core::vocab::{
    Align2 as MaraAlign2, Pos2 as MaraPos2, Stroke as MaraStroke, TextureId as MaraTextureId,
};
use mara_core::{ViewCtx, vocab::Color32 as MaraColor32, vocab::Rect as MaraRect};

use crate::{BevyEmbeddedView, BevyViewportInput, BevyViewportWgpuResources};

/// Egui/Mara-hosted Bevy viewport.
///
/// The host still owns the top-level window. This widget reserves an
/// egui region, asks the embedded Bevy bridge to render into an
/// offscreen target, uploads the latest RGBA frame as an egui texture,
/// and forwards pointer/scroll interaction into the Bevy camera.
pub struct MaraBevyViewport {
    /// Per-instance salt for egui area/interact ids, so two viewports
    /// in one context never collide on shared area memory or input.
    instance: u64,
    bevy: BevyEmbeddedView,
    texture: Option<egui::TextureHandle>,
    last_pixels: [u32; 2],
    resize_target_pixels: [u32; 2],
    resize_settle_until: f64,
    last_render_time: f64,
    continuous_rendering: bool,
    last_pointer_pos: Option<egui::Pos2>,
    primary_drag_active: bool,
    native_texture: Option<egui::TextureId>,
    native_texture_size: [usize; 2],
}

impl Default for MaraBevyViewport {
    fn default() -> Self {
        Self::new()
    }
}

/// Monotonic per-instance salt so every viewport gets unique egui ids.
fn next_viewport_instance() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl MaraBevyViewport {
    pub fn new() -> Self {
        Self {
            instance: next_viewport_instance(),
            bevy: BevyEmbeddedView::new(),
            texture: None,
            last_pixels: [0, 0],
            resize_target_pixels: [0, 0],
            resize_settle_until: f64::NEG_INFINITY,
            last_render_time: f64::NEG_INFINITY,
            continuous_rendering: false,
            last_pointer_pos: None,
            primary_drag_active: false,
            native_texture: None,
            native_texture_size: [0, 0],
        }
    }

    pub fn with_content(
        configure_app: impl Fn(&mut bevy::prelude::App) + Send + Sync + 'static,
    ) -> Self {
        Self {
            instance: next_viewport_instance(),
            bevy: BevyEmbeddedView::with_app_config(configure_app),
            texture: None,
            last_pixels: [0, 0],
            resize_target_pixels: [0, 0],
            resize_settle_until: f64::NEG_INFINITY,
            last_render_time: f64::NEG_INFINITY,
            continuous_rendering: false,
            last_pointer_pos: None,
            primary_drag_active: false,
            native_texture: None,
            native_texture_size: [0, 0],
        }
    }

    /// A viewport that takes its wgpu resources at construction.
    ///
    /// Takes the opaque [`MaraRenderState`](mara_gpu::MaraRenderState)
    /// rather than a raw `egui_wgpu::RenderState` (PLAN.md WS-C1.4), so
    /// an app can build one at startup from
    /// `CreationContext::gpu()` without naming a backend type.
    pub fn with_render_state(render_state: Option<mara_gpu::MaraRenderState<'_>>) -> Self {
        let bevy = render_state
            .map(|render_state| {
                let render_state = render_state.__internal_raw();
                BevyEmbeddedView::with_wgpu_resources(BevyViewportWgpuResources::new(
                    render_state.device.clone(),
                    render_state.queue.clone(),
                    render_state.adapter.clone(),
                ))
            })
            .unwrap_or_default();
        Self {
            instance: next_viewport_instance(),
            bevy,
            texture: None,
            last_pixels: [0, 0],
            resize_target_pixels: [0, 0],
            resize_settle_until: f64::NEG_INFINITY,
            last_render_time: f64::NEG_INFINITY,
            continuous_rendering: false,
            last_pointer_pos: None,
            primary_drag_active: false,
            native_texture: None,
            native_texture_size: [0, 0],
        }
    }

    /// [`with_render_state`](Self::with_render_state) with an app
    /// configurator, for a viewport whose Bevy world needs setting up.
    pub fn with_render_state_and_content(
        render_state: Option<mara_gpu::MaraRenderState<'_>>,
        configure_app: impl Fn(&mut bevy::prelude::App) + Send + Sync + 'static,
    ) -> Self {
        let bevy = if let Some(render_state) = render_state {
            let render_state = render_state.__internal_raw();
            BevyEmbeddedView::with_wgpu_resources_and_app_config(
                BevyViewportWgpuResources::new(
                    render_state.device.clone(),
                    render_state.queue.clone(),
                    render_state.adapter.clone(),
                ),
                configure_app,
            )
        } else {
            BevyEmbeddedView::with_app_config(configure_app)
        };
        Self {
            instance: next_viewport_instance(),
            bevy,
            texture: None,
            last_pixels: [0, 0],
            resize_target_pixels: [0, 0],
            resize_settle_until: f64::NEG_INFINITY,
            last_render_time: f64::NEG_INFINITY,
            continuous_rendering: false,
            last_pointer_pos: None,
            primary_drag_active: false,
            native_texture: None,
            native_texture_size: [0, 0],
        }
    }

    /// Pause/resume the embedded Bevy renderer.
    ///
    /// Call this from the host when the Bevy viewport is not the
    /// active Mara view. The last texture is retained, but Bevy's
    /// offscreen app is not ticked and its cameras are marked
    /// inactive until the view is active again.
    pub fn set_active(&mut self, active: bool) {
        self.bevy.set_rendering_enabled(active);
        if !active {
            self.primary_drag_active = false;
            self.last_pointer_pos = None;
        }
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.bevy.rendering_enabled()
    }

    /// The embedded app's world, once the renderer exists (after the
    /// first `show`). Hosts drive their panes against it between frames.
    pub fn world_mut(&mut self) -> Option<&mut bevy::prelude::World> {
        self.bevy.world_mut()
    }

    /// Like [`Self::with_render_state_and_content`], with a hook that adjusts
    /// the embedded app's `DefaultPlugins` before they are added.
    pub fn with_render_state_plugins_and_content(
        render_state: Option<mara_gpu::MaraRenderState<'_>>,
        configure_plugins: impl Fn(bevy::app::PluginGroupBuilder) -> bevy::app::PluginGroupBuilder
        + Send
        + Sync
        + 'static,
        configure_app: impl Fn(&mut bevy::prelude::App) + Send + Sync + 'static,
    ) -> Self {
        let mut view = Self::with_render_state_and_content(render_state, configure_app);
        view.bevy.set_plugins_config(configure_plugins);
        view
    }

    /// Use the active frame rate for animated content without pointer input.
    pub fn set_continuous_rendering(&mut self, enabled: bool) {
        self.continuous_rendering = enabled;
    }

    pub fn show(
        &mut self,
        ctx: &mut ViewCtx<'_>,
        render_state: Option<mara_gpu::MaraRenderState<'_>>,
        accent: impl Into<MaraColor32>,
    ) -> Option<MaraColor32> {
        let render_state: Option<&egui_wgpu::RenderState> =
            render_state.map(|state| state.__internal_raw());
        let accent = accent.into();
        self.set_active(true);
        let mut picked_color = None;
        let region_rect: egui::Rect = ctx.screen_rect().into();
        {
            // The surface comes from `ViewCtx::body_at` rather than a
            // hand-built `egui::Area` (PLAN.md WS-C1.3). `Layer::Background`
            // maps to `Order::Background`, the origin is still fixed at the
            // region's min, and areas stay interactable — the body below
            // relies on that for its own `interact` call.
            //
            // One deliberate difference: the area id is now scoped to the
            // workspace (`body_at` salts with `workspace.current().id`), so
            // two workspaces each showing a viewport no longer share one
            // area's state. `movable` also goes explicitly false, which the
            // fixed position already made true in practice.
            //
            // The body still takes a raw `Ui`: it registers wgpu textures
            // and drives an `egui_wgpu` render state, host-tier work the
            // seam deliberately does not model.
            ctx.body_at(
                ("mara_bevy_viewport_area", self.instance),
                MaraRect::from(region_rect),
                Layer::Background,
                |mara| {
                    let rect = region_rect;
                    // Taken before the raw `Ui`, which borrows `mara` for
                    // the rest of the body. `with_clip` is the seam's
                    // `painter_at`.
                    let painter = mara.painter().with_clip(MaraRect::from(rect));
                    let ui = mara.__internal_raw_ui();
                    ui.set_clip_rect(region_rect);
                    let theme = mara_core::style::theme();
                    painter.rect_filled(rect, 0.0, theme.palette.bg_panel);
                    if rect.width() < 16.0 || rect.height() < 16.0 {
                        if let Some(texture_id) = self.native_texture {
                            paint_texture_id_cover(
                                &painter,
                                texture_id.into(),
                                self.native_texture_size,
                                rect.into(),
                            );
                        } else if let Some(texture) = &self.texture {
                            paint_texture_cover(&painter, texture, rect.into());
                        }
                        ui.ctx()
                            .request_repaint_after(Duration::from_secs_f64(1.0 / 12.0));
                        return;
                    }

                    if let Some(render_state) = render_state {
                        self.bevy
                            .attach_wgpu_resources(BevyViewportWgpuResources::new(
                                render_state.device.clone(),
                                render_state.queue.clone(),
                                render_state.adapter.clone(),
                            ));
                    }
                    let use_native_gpu_texture = render_state.is_some();

                    let response = ui.interact(
                        rect,
                        egui::Id::unique(("mara_embedded_bevy_viewport_interact", self.instance)),
                        egui::Sense::click_and_drag(),
                    );
                    let ppp = ui.ctx().pixels_per_point();
                    let now = MaraCtx::now(&EguiCtx::new(ui.ctx()));
                    let target_pixels = internal_render_pixels(rect.size(), ppp);
                    if self.resize_target_pixels != target_pixels {
                        self.resize_target_pixels = target_pixels;
                        self.resize_settle_until = now + resize_settle_seconds();
                    }
                    let has_committed_texture =
                        self.native_texture.is_some() || self.texture.is_some();
                    let waiting_for_resize_settle = has_committed_texture
                        && self.last_pixels != [0, 0]
                        && self.last_pixels != target_pixels
                        && now < self.resize_settle_until;
                    let pixels = if waiting_for_resize_settle {
                        self.last_pixels
                    } else {
                        target_pixels
                    };
                    let resize_pending = self.last_pixels != [0, 0] && self.last_pixels != pixels;
                    let render_scale = egui::vec2(
                        pixels[0] as f32 / rect.width().max(1.0),
                        pixels[1] as f32 / rect.height().max(1.0),
                    );

                    // Only the viewport's own egui `Response` may start
                    // Bevy interaction. Do NOT use raw/global pointer
                    // containment as the start condition: floating menus,
                    // panes and container-dot handles can sit above this
                    // rect, so `rect.contains(pointer)` would leak those
                    // UI clicks into the Bevy camera/picker below.
                    let viewport_hovered = response.hovered();
                    let pointer_pos = if viewport_hovered || self.primary_drag_active {
                        MaraCtx::input(&EguiCtx::new(ui.ctx()))
                            .interact_pointer
                            .map(Into::into)
                    } else {
                        response.hover_pos()
                    };
                    // Through `MaraInput` rather than a raw egui input
                    // closure — every flag this used to hand-roll now
                    // exists on the seam (PLAN.md WS-C1.3). The `/120.0`
                    // stays explicit: `MaraInput::scroll_delta` is the
                    // raw smooth delta in points, and this viewport wants
                    // wheel *notches*.
                    let seam_input = MaraCtx::input(&EguiCtx::new(ui.ctx()));
                    let primary_down = seam_input.primary_down;
                    let primary_pressed = seam_input.primary_pressed;
                    let middle_down = seam_input.middle_down;
                    let middle_pressed = seam_input.middle_pressed;
                    let scroll_delta = if viewport_hovered {
                        seam_input.scroll_delta.y / 120.0
                    } else {
                        0.0
                    };
                    let viewport_drag_started =
                        viewport_hovered && (primary_pressed || middle_pressed);
                    if viewport_drag_started {
                        self.primary_drag_active = true;
                        self.last_pointer_pos = pointer_pos;
                    }
                    let viewport_drag_down = primary_down || middle_down;
                    let viewport_dragged = viewport_drag_down && self.primary_drag_active;
                    let pointer_delta = if viewport_dragged {
                        if let Some(pos) = pointer_pos {
                            let delta = self
                                .last_pointer_pos
                                .map(|last| pos - last)
                                .unwrap_or_default()
                                * render_scale;
                            self.last_pointer_pos = Some(pos);
                            [delta.x, delta.y]
                        } else {
                            [0.0, 0.0]
                        }
                    } else {
                        if !viewport_drag_down {
                            self.primary_drag_active = false;
                            self.last_pointer_pos = None;
                        }
                        [0.0, 0.0]
                    };
                    let middle_dragged = viewport_dragged && middle_down;
                    let primary_dragged = viewport_dragged && primary_down && !middle_dragged;
                    let drag_delta = if primary_dragged {
                        pointer_delta
                    } else {
                        [0.0, 0.0]
                    };
                    let pan_delta = if middle_dragged {
                        pointer_delta
                    } else {
                        [0.0, 0.0]
                    };
                    let primary_clicked = primary_pressed && viewport_hovered;

                    let viewport_input = BevyViewportInput {
                        pointer_pos: pointer_pos.map(|pos| {
                            [
                                ((pos.x - rect.left()) * render_scale.x)
                                    .clamp(0.0, pixels[0] as f32),
                                ((pos.y - rect.top()) * render_scale.y)
                                    .clamp(0.0, pixels[1] as f32),
                            ]
                        }),
                        drag_delta,
                        pan_delta,
                        scroll_delta,
                        primary_clicked,
                    };

                    let input_active = viewport_dragged
                        || primary_clicked
                        || response.hovered() && viewport_input.scroll_delta.abs() > f32::EPSILON
                        // Still raw: `MaraInput` has no `any_down`, and
                        // composing it from primary/secondary/middle would
                        // silently drop egui's Extra1/Extra2 buttons.
                        || response.hovered() && ui.ctx().input(|i| i.pointer.any_down());
                    let target_size = [pixels[0] as usize, pixels[1] as usize];
                    let native_texture_needs_committed_frame =
                        self.native_texture.is_some() && self.native_texture_size != target_size;
                    let cpu_texture_needs_committed_frame = self
                        .texture
                        .as_ref()
                        .is_some_and(|texture| texture.size() != target_size);
                    let texture_needs_committed_frame =
                        native_texture_needs_committed_frame || cpu_texture_needs_committed_frame;
                    let has_texture = has_committed_texture;
                    #[cfg(target_arch = "wasm32")]
                    let idle_interval = 1.0 / 12.0;
                    #[cfg(not(target_arch = "wasm32"))]
                    let idle_interval = 1.0 / 24.0;
                    #[cfg(target_arch = "wasm32")]
                    let active_interval = 1.0 / 30.0;
                    #[cfg(not(target_arch = "wasm32"))]
                    let active_interval = 1.0 / 60.0;
                    let target_interval = frame_interval(self.continuous_rendering, input_active
                        || resize_pending
                        || texture_needs_committed_frame
                        || !has_texture, active_interval, idle_interval);
                    let elapsed = now - self.last_render_time;
                    let should_render = resize_pending
                        || !has_texture
                        || texture_needs_committed_frame
                        || input_active
                        || elapsed >= target_interval;
                    let render_started = bevy::platform::time::Instant::now();

                    if should_render {
                        self.last_render_time = now;
                        // Still raw: `MaraCtx::dt()` is egui's
                        // *unstable* dt. This is a render-pacing value
                        // and wants the smoothed one, so substituting
                        // would change behaviour rather than relocate it.
                        let dt = ui.ctx().input(|i| i.stable_dt);
                        let render_attempts = 1;
                        for attempt in 0..render_attempts {
                            let input = if attempt == 0 {
                                viewport_input
                            } else {
                                BevyViewportInput {
                                    pointer_pos: viewport_input.pointer_pos,
                                    ..Default::default()
                                }
                            };
                            let dt = if attempt == 0 { dt } else { 0.0 };
                            if use_native_gpu_texture
                                && let Some(render_state) = render_state
                                && let Some(frame) = self
                                    .bevy
                                    .render_texture_with_input(pixels[0], pixels[1], dt, input)
                            {
                                let size = [frame.width as usize, frame.height as usize];
                                if size == target_size {
                                    let texture_id = {
                                        let mut renderer = render_state.renderer.write();
                                        match self.native_texture {
                                            Some(texture_id)
                                                if self.native_texture_size == size =>
                                            {
                                                // The egui texture id already points at the
                                                // Bevy render target. Bevy updates the GPU
                                                // texture contents in-place, so avoid rebuilding
                                                // the egui bind group every frame.
                                                texture_id
                                            }
                                            Some(texture_id) => {
                                                renderer.free_texture(&texture_id);
                                                renderer.register_native_texture(
                                                    &render_state.device,
                                                    &frame.view,
                                                    wgpu::FilterMode::Linear,
                                                )
                                            }
                                            None => renderer.register_native_texture(
                                                &render_state.device,
                                                &frame.view,
                                                wgpu::FilterMode::Linear,
                                            ),
                                        }
                                    };
                                    self.native_texture = Some(texture_id);
                                    self.native_texture_size = size;
                                    self.last_pixels = pixels;
                                    break;
                                }
                            }

                            if !use_native_gpu_texture
                                && let Some(frame) = self
                                    .bevy
                                    .render_frame_with_input(pixels[0], pixels[1], dt, input)
                            {
                                let size = [frame.width as usize, frame.height as usize];
                                let rgba = if size == target_size {
                                    Some(frame.rgba.clone())
                                } else {
                                    None
                                };
                                if let Some(rgba) = rgba {
                                    self.last_pixels = pixels;
                                    let image =
                                        egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                                    match &mut self.texture {
                                        Some(texture) if texture.size() == size => {
                                            texture.set(image, egui::TextureOptions::LINEAR);
                                        }
                                        _ => {
                                            self.texture = Some(ui.ctx().load_texture(
                                                "mara_embedded_bevy_viewport",
                                                image,
                                                egui::TextureOptions::LINEAR,
                                            ));
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                    }

                    if let Some(texture_id) = self.native_texture {
                        paint_texture_id_cover(
                            &painter,
                            texture_id.into(),
                            self.native_texture_size,
                            rect.into(),
                        );
                    } else if let Some(texture) = &self.texture {
                        paint_texture_cover(&painter, texture, rect.into());
                    } else if self.texture.is_none() {
                        self.paint_warmup(&painter, rect.into(), accent);
                    }

                    picked_color = self.bevy.picked_color();
                    let spent = if should_render {
                        render_started.elapsed().as_secs_f64()
                    } else {
                        elapsed
                    };
                    let mut next = next_frame_delay(target_interval, spent);
                    if resize_pending || texture_needs_committed_frame {
                        next = next.min(active_interval);
                    }
                    if waiting_for_resize_settle {
                        next = next.min((self.resize_settle_until - now).max(0.0));
                    }
                    ui.ctx()
                        .request_repaint_after(repaint_delay(next));
                },
            );
        }
        picked_color
    }

    /// The placeholder grid shown until Bevy's first frame lands.
    ///
    /// Entirely `MaraPainter` (PLAN.md WS-C1.3) — lines, text and a
    /// colour, none of which needs a backend type.
    fn paint_warmup(&self, painter: &MaraPainter, rect: MaraRect, accent: MaraColor32) {
        let grid = 36.0;
        let grid_col = MaraColor32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 28);
        let stroke = MaraStroke::new(1.0, grid_col);
        let mut x = rect.left();
        while x < rect.right() {
            painter.line_segment(
                MaraPos2::new(x, rect.top()),
                MaraPos2::new(x, rect.bottom()),
                stroke,
            );
            x += grid;
        }
        let mut y = rect.top();
        while y < rect.bottom() {
            painter.line_segment(
                MaraPos2::new(rect.left(), y),
                MaraPos2::new(rect.right(), y),
                stroke,
            );
            y += grid;
        }
        painter.text(
            rect.center(),
            MaraAlign2::CENTER_CENTER,
            if self.bevy.renderer_failed() {
                "embedded Bevy renderer unavailable: no wgpu adapter"
            } else {
                "warming up embedded Bevy renderer…"
            },
            13.0,
            mara_core::style::on_panel(),
        );
    }
}

fn frame_interval(continuous: bool, interactive: bool, active: f64, idle: f64) -> f64 {
    if continuous || interactive { active } else { idle }
}

fn next_frame_delay(interval: f64, spent: f64) -> f64 {
    (interval - spent).max(0.0)
}

fn repaint_delay(seconds: f64) -> Duration {
    let interval = Duration::try_from_secs_f64(seconds).unwrap_or_default();
    if interval.is_zero() {
        return Duration::ZERO;
    }
    interval
}

#[cfg(test)]
mod pacing_tests {
    #[test]
    fn animation_uses_active_rate_without_input() {
        for (active, idle) in [(1.0 / 60.0, 1.0 / 24.0), (1.0 / 30.0, 1.0 / 12.0)] {
            assert_eq!(super::frame_interval(false, false, active, idle), idle);
            assert_eq!(super::frame_interval(true, false, active, idle), active);
            assert_eq!(super::frame_interval(false, true, active, idle), active);
            assert_eq!(super::frame_interval(true, true, active, idle), active);
        }
    }
    use super::*;

    #[test]
    fn frame_cost_comes_out_of_the_interval() {
        let interval = 1.0 / 60.0;
        assert!((next_frame_delay(interval, 0.005) - (interval - 0.005)).abs() < 1e-12);
        assert_eq!(next_frame_delay(interval, 0.018), 0.0);
        assert_eq!(next_frame_delay(interval, 0.0), interval);
        assert_eq!(repaint_delay(next_frame_delay(interval, 0.018)), Duration::ZERO);
    }

    #[test]
    fn repaint_interval_survives_egui_prediction() {
        for predicted_dt in [0.0, 1.0 / 60.0, 1.0 / 144.0] {
            let ctx = egui::Context::default();
            let mut output = egui::FullOutput::default();
            for _ in 0..5 {
                output = ctx.run_ui(egui::RawInput { predicted_dt, ..Default::default() }, |ui| {
                    ui.ctx().request_repaint_after(repaint_delay(1.0 / 60.0));
                });
                // Headless: nothing uploads the texture deltas.
                output.textures_delta.clear();
            }
            let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            assert!((Duration::from_millis(16)..=Duration::from_millis(17)).contains(&delay));
        }
    }

    #[test]
    fn immediate_and_invalid_intervals_remain_immediate() {
        for seconds in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(repaint_delay(seconds), Duration::ZERO);
        }
        assert_eq!(repaint_delay(0.1), Duration::from_millis(100));
    }
}

fn internal_render_pixels(size: egui::Vec2, pixels_per_point: f32) -> [u32; 2] {
    // Keep the embedded Bevy view close to native DPI. The old cap
    // was intentionally conservative while debugging the web bridge,
    // but it forced high-DPI/browser windows to render low-res and
    // then upscale in egui, making the scene visibly soft.
    #[cfg(not(target_arch = "wasm32"))]
    const MAX_WIDTH: f32 = 2560.0;
    #[cfg(target_arch = "wasm32")]
    const MAX_WIDTH: f32 = 1920.0;
    #[cfg(not(target_arch = "wasm32"))]
    const MAX_HEIGHT: f32 = 1600.0;
    #[cfg(target_arch = "wasm32")]
    const MAX_HEIGHT: f32 = 1200.0;
    #[cfg(not(target_arch = "wasm32"))]
    const MAX_PIXELS: f32 = 3_600_000.0;
    #[cfg(target_arch = "wasm32")]
    const MAX_PIXELS: f32 = 1_600_000.0;

    let mut width = (size.x * pixels_per_point).round().max(1.0);
    let mut height = (size.y * pixels_per_point).round().max(1.0);

    let scale = (MAX_WIDTH / width)
        .min(MAX_HEIGHT / height)
        .min((MAX_PIXELS / (width * height)).sqrt())
        .min(1.0);
    width = (width * scale).round().max(1.0);
    height = (height * scale).round().max(1.0);

    [width as u32, height as u32]
}

fn resize_settle_seconds() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        0.10
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0.07
    }
}

/// Draw `texture` filling `rect`, cropping rather than stretching.
fn paint_texture_cover(painter: &MaraPainter, texture: &egui::TextureHandle, rect: MaraRect) {
    paint_texture_id_cover(painter, texture.id().into(), texture.size(), rect);
}

/// [`paint_texture_cover`] for a texture the host registered itself.
///
/// Through `MaraPainter` (PLAN.md WS-C1.3): the aspect-fit maths was
/// already backend-neutral, and `MaraPainter::image` takes the same
/// texture/uv/tint triple the raw painter did.
fn paint_texture_id_cover(
    painter: &MaraPainter,
    texture_id: MaraTextureId,
    texture_size: [usize; 2],
    rect: MaraRect,
) {
    let [texture_width, texture_height] = texture_size;
    if texture_width == 0 || texture_height == 0 || !(rect.width() > 0.0 && rect.height() > 0.0) {
        return;
    }

    let texture_aspect = texture_width as f32 / texture_height as f32;
    let rect_aspect = rect.width() / rect.height().max(1.0);
    let uv = if rect_aspect > texture_aspect {
        // The viewport is wider than the old frame. Cover the rect by
        // cropping top/bottom instead of stretching.
        let visible_v = (texture_aspect / rect_aspect).clamp(0.0, 1.0);
        let pad_v = (1.0 - visible_v) * 0.5;
        MaraRect::from_min_max(MaraPos2::new(0.0, pad_v), MaraPos2::new(1.0, 1.0 - pad_v))
    } else {
        // The viewport is taller/narrower than the old frame. Cover
        // the rect by cropping left/right instead of stretching.
        let visible_u = (rect_aspect / texture_aspect).clamp(0.0, 1.0);
        let pad_u = (1.0 - visible_u) * 0.5;
        MaraRect::from_min_max(MaraPos2::new(pad_u, 0.0), MaraPos2::new(1.0 - pad_u, 1.0))
    };

    painter.image(texture_id, rect, uv, MaraColor32::WHITE);
}
