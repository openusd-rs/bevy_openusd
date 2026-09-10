use bevy::{camera::RenderTarget, prelude::*};
use mara::{host::MaraHostCtx, ui::{mara_core, modules::bevy as mara_bevy},
    window::{CreationContext, WindowApp}};
use mara_core::{RibbonAvoidance, WorkspaceStack, style::{AccentColor, GlassOpacity, active_accent}};

struct Probe {
    viewport: mara_bevy::MaraBevyViewport,
    workspace: WorkspaceStack,
    context: eframe::egui::Context,
    announced: bool,
    cpu_readback: bool,
}

fn cpu_readback(value: Option<&str>) -> Result<bool, &'static str> {
    match value {
        None | Some("native") => Ok(false),
        Some("cpu") => Ok(true),
        _ => Err("USD_BRIDGE_PROBE_TRANSFER must be native or cpu"),
    }
}

impl WindowApp for Probe {
    fn new(ctx: CreationContext<'_>) -> Self {
        let cpu_readback = cpu_readback(std::env::var("USD_BRIDGE_PROBE_TRANSFER").ok().as_deref()).unwrap();
        eprintln!("BRIDGE_CAPTURE_PROBE Bevy viewport, no USD plugins or editor, cpu_readback={cpu_readback}");
        Self {
            viewport: if cpu_readback { mara_bevy::MaraBevyViewport::with_content(configure) }
                else { mara_bevy::MaraBevyViewport::with_render_state_and_content(ctx.gpu(), configure) },
            workspace: WorkspaceStack::new("bridge-probe"),
            context: ctx.__internal_egui_ctx().clone(),
            announced: false,
            cpu_readback,
        }
    }

    fn update(&mut self, host: &mut MaraHostCtx<'_>) {
        host.apply_theme(AccentColor::default(), GlassOpacity::default());
        let accent = active_accent();
        let mut view = host.view_ctx(&mut self.workspace, accent, RibbonAvoidance::all());
        view.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
        self.viewport.show(&mut view, if self.cpu_readback { None } else { host.gpu() }, accent);
        let painter = self.context.layer_painter(eframe::egui::LayerId::new(
            eframe::egui::Order::Foreground, eframe::egui::Id::new("bridge-probe-label")));
        painter.text(eframe::egui::pos2(32.0, 32.0), eframe::egui::Align2::LEFT_TOP,
            "Mara + Bevy bridge only - no USD", eframe::egui::FontId::proportional(28.0),
            eframe::egui::Color32::WHITE);
        if !self.announced {
            eprintln!("USD_VIEWER_UI_UPDATED");
            self.announced = true;
        }
    }
}

fn configure(app: &mut App) {
    app.insert_resource(ClearColor(Color::srgb_u8(24, 48, 96)))
        .add_systems(Startup, setup.after(mara_bevy::BevyViewportSet::SetupTarget));
}

fn setup(mut commands: Commands, target: Res<mara_bevy::BevyViewportRenderTarget>,
    mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.spawn((Camera3d::default(), RenderTarget::from(target.0.clone()),
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y)));
    commands.spawn((Mesh3d(meshes.add(Cuboid::new(2.0, 2.0, 2.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb_u8(240, 100, 24), unlit: true, ..default()
        })), Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, 0.3, 0.5, 0.0))));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let transfer = std::env::var("USD_BRIDGE_PROBE_TRANSFER");
    if matches!(transfer, Err(std::env::VarError::NotUnicode(_))) {
        return Err("USD_BRIDGE_PROBE_TRANSFER must be Unicode".into());
    }
    cpu_readback(transfer.ok().as_deref())?;
    mara::window::run::<Probe>()
}

#[test]
fn bridge_transfer_modes_are_explicit() {
    assert_eq!(cpu_readback(None), Ok(false));
    assert_eq!(cpu_readback(Some("native")), Ok(false));
    assert_eq!(cpu_readback(Some("cpu")), Ok(true));
    for invalid in ["", "true", "CPU", "gpu"] {
        assert!(cpu_readback(Some(invalid)).is_err());
    }
}

#[test]
fn bridge_probe_builds_only_a_camera_and_unlit_mesh() {
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>()
        .insert_resource(mara_bevy::BevyViewportRenderTarget(Handle::default()));
    configure(&mut app);
    app.update();
    let world = app.world_mut();
    assert_eq!(world.query::<&Camera3d>().iter(world).count(), 1);
    assert_eq!(world.query::<&Mesh3d>().iter(world).count(), 1);
    let material = world.query::<&MeshMaterial3d<StandardMaterial>>().single(world).unwrap();
    let material = world.resource::<Assets<StandardMaterial>>().get(&material.0).unwrap();
    assert!(material.unlit);
    assert_eq!(material.base_color, Color::srgb_u8(240, 100, 24));
    assert_eq!(world.resource::<ClearColor>().0, Color::srgb_u8(24, 48, 96));
}
