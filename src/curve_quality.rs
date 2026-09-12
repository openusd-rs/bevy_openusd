use usd_bevy::route::curves::UsdCurveSettings;

pub fn from_env() -> Result<UsdCurveSettings, String> {
    let settings = match std::env::var("USD_CURVE_STEPS") {
        Ok(value) => parse(Some(&value)),
        Err(std::env::VarError::NotPresent) => parse(None),
        Err(error) => Err(format!("USD_CURVE_STEPS: {error}")),
    }?;
    match std::env::var("USD_CURVE_SURFACE_SIDES") {
        Ok(value) => parse_surface(settings, Some(&value)),
        Err(std::env::VarError::NotPresent) => Ok(settings),
        Err(error) => Err(format!("USD_CURVE_SURFACE_SIDES: {error}")),
    }
}

fn parse_surface(settings: UsdCurveSettings, value: Option<&str>) -> Result<UsdCurveSettings, String> {
    let sides = value.map(|value| value.parse().map_err(|_| "USD_CURVE_SURFACE_SIDES must be an integer in 3..=32")).transpose()?;
    settings.with_surface_sides(sides).map_err(|error| format!("USD_CURVE_SURFACE_SIDES: {error}"))
}

#[test]
fn surface_quality_configuration_is_bounded() {
    assert_eq!(parse_surface(UsdCurveSettings::default(), None).unwrap().surface_sides(), None);
    for sides in [3,12,32] { assert_eq!(parse_surface(UsdCurveSettings::default(), Some(&sides.to_string())).unwrap().surface_sides(), Some(sides)); }
    for value in ["", "0", "2", "33", "-1", "1.5"] { assert!(parse_surface(UsdCurveSettings::default(), Some(value)).is_err()); }
}

fn parse(value: Option<&str>) -> Result<UsdCurveSettings, String> {
    let Some(value) = value else { return Ok(UsdCurveSettings::default()); };
    let steps = value.parse().map_err(|_| "USD_CURVE_STEPS must be an integer in 1..=64")?;
    UsdCurveSettings::new(steps).map_err(|error| format!("USD_CURVE_STEPS: {error}"))
}

#[test]
fn curve_quality_configuration_is_bounded() {
    assert_eq!(parse(None).unwrap().cubic_steps(), 8);
    for steps in [1, 8, 32, 64] {
        assert_eq!(parse(Some(&steps.to_string())).unwrap().cubic_steps(), steps);
    }
    for value in ["", "0", "65", "-1", "1.5", "auto", "9999999999999999999999999"] {
        assert!(parse(Some(value)).unwrap_err().contains("USD_CURVE_STEPS"));
    }
}
