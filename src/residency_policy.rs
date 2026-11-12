use bevy::prelude::*;
use usd_bevy::route::residency::DeferHiddenMeshes;

fn parse(value: Option<&str>) -> Result<bool, String> {
    match value {
        Some("0") => Ok(false),
        None | Some("1") => Ok(true),
        _ => Err("USD_DEFER_HIDDEN_MESHES must be 0 or 1".into()),
    }
}

pub fn from_env() -> Result<bool, String> {
    match std::env::var("USD_DEFER_HIDDEN_MESHES") {
        Ok(value) => parse(Some(&value)),
        Err(std::env::VarError::NotPresent) => parse(None),
        Err(error) => Err(error.to_string()),
    }
}

pub fn configure(app: &mut App, enabled: bool) {
    if enabled {
        app.init_resource::<DeferHiddenMeshes>();
    }
}

#[test]
fn viewer_residency_defers_hidden_meshes_unless_disabled() {
    for (value, expected) in [(None, true), (Some("0"), false), (Some("1"), true)] {
        let enabled = parse(value).unwrap();
        assert_eq!(enabled, expected);
        let mut app = App::new();
        configure(&mut app, enabled);
        assert_eq!(
            app.world().contains_resource::<DeferHiddenMeshes>(),
            expected
        );
    }
    for value in ["", "true", "false", "2", " 1", "01"] {
        assert!(parse(Some(value)).is_err());
    }
}
