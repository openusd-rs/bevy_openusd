use bevy::prelude::*;
use usd_bevy::route::residency::DeferHiddenMeshes;

fn parse(value: Option<&str>) -> Result<bool, String> {
    match value {
        None | Some("0") => Ok(false),
        Some("1") => Ok(true),
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
fn viewer_residency_is_eager_unless_explicitly_enabled() {
    for (value, expected) in [(None, false), (Some("0"), false), (Some("1"), true)] {
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
