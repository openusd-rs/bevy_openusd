use usd_bevy::route::curves::UsdCurveSettings;

pub fn from_env() -> Result<UsdCurveSettings, String> {
    match std::env::var("USD_CURVE_STEPS") {
        Ok(value) => parse(Some(&value)),
        Err(std::env::VarError::NotPresent) => parse(None),
        Err(error) => Err(format!("USD_CURVE_STEPS: {error}")),
    }
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
