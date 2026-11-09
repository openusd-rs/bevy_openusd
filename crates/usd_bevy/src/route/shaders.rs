use bevy::{platform::collections::HashMap, prelude::*};

#[derive(Resource, Default)]
struct Libraries(HashMap<&'static str, Handle<Shader>>);

pub(super) fn retain(app: &mut App, path: &'static str) {
    if !app.world().contains_resource::<Assets<Shader>>() {
        return;
    }
    app.init_resource::<Libraries>();
    let server = app.world().resource::<AssetServer>().clone();
    app.world_mut()
        .resource_mut::<Libraries>()
        .0
        .entry(path)
        .or_insert_with(|| server.load(path));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_registration_precedes_material_plugin_and_runs_once() {
        use super::super::{
            flat_material::{FlatMaterial, FlatNormals},
            wrapped,
        };
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        wrapped::configure::<FlatNormals>(&mut app, |app| {
            assert!(!app.is_plugin_added::<bevy::pbr::MaterialPlugin<FlatMaterial>>());
            assert!(!app.world().contains_resource::<Assets<FlatMaterial>>());
        });
        assert!(app.is_plugin_added::<bevy::pbr::MaterialPlugin<FlatMaterial>>());
        wrapped::configure::<FlatNormals>(&mut app, |_| panic!("registered shaders twice"));
    }

    #[test]
    fn material_and_instancing_libraries_load_and_stay_retained() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_asset::<Shader>()
            .init_asset_loader::<bevy::shader::ShaderLoader>()
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>();
        for _ in 0..2 {
            super::super::flat_material::configure(&mut app);
            super::super::strand_material::configure(&mut app);
            super::super::sheen_material::configure(&mut app);
            super::super::medium_material::configure(&mut app);
        }
        app.add_plugins(super::super::gpu_instancing::UsdGpuInstancingPlugin);
        assert_eq!(app.world().resource::<Libraries>().0.len(), 4);
        let server = app.world().resource::<AssetServer>().clone();
        let entries = [
            "flat_material",
            "flat_prepass",
            "strand_material",
            "strand_prepass",
            "sheen_material",
            "medium_material",
            "gpu_instancing",
            "gpu_instancing_shadow",
        ]
        .map(|name| server.load::<Shader>(format!("embedded://usd_bevy/route/{name}.wesl")));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            app.update();
            let assets = app.world().resource::<Assets<Shader>>();
            if entries.iter().all(|handle| assets.contains(handle))
                && app
                    .world()
                    .resource::<Libraries>()
                    .0
                    .values()
                    .all(|handle| assets.contains(handle))
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "shader assets did not load"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        for _ in 0..4 {
            app.update();
        }
        let assets = app.world().resource::<Assets<Shader>>();
        for (path, handle) in &app.world().resource::<Libraries>().0 {
            assert_eq!(assets.get(handle).unwrap().path, *path);
        }
    }
}
