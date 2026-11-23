//! Read-only OCPP 1.6J charging telemetry. The caller supplies the station identity
//! from the connection or recording and the instant being compared.
use ocpp_kit::v1_6::{Location, Measurand, UnitOfMeasure, ValueFormat};
pub use ocpp_kit::{
    types::DateTime,
    v1_6::{ChargingProfileStatus, MeterValuesRequest, SetChargingProfileResponse},
};

/// Whole-station active import in kW at the specified instant, for a site that
/// commissioned connector zero's inlet as the station total. Absent, invalid
/// and ambiguous readings all leave the contribution unknown.
pub fn station_import_kw(requests: &[MeterValuesRequest], at: DateTime) -> Option<f64> {
    let mut samples = requests
        .iter()
        .filter(|r| r.connector_id == 0)
        .flat_map(|r| &r.meter_value)
        .filter(|r| r.timestamp == at)
        .flat_map(|r| &r.sampled_value)
        .filter(|s| {
            s.measurand == Some(Measurand::PowerActiveImport)
                && s.location == Some(Location::Inlet)
                && s.phase.is_none()
                && matches!(s.format, None | Some(ValueFormat::Raw))
        });
    let sample = samples.next()?;
    if samples.next().is_some() {
        return None;
    }
    let value = sample.value.parse::<f64>().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    match sample.unit {
        Some(UnitOfMeasure::W) => Some(value / 1000.0),
        Some(UnitOfMeasure::KW) => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_one_aligned_raw_station_inlet_can_contribute() {
        let time = "2026-11-22T08:12:15Z";
        let at = DateTime::parse(time).unwrap();
        let sample = json!({"value":"12500","measurand":"Power.Active.Import","unit":"W","location":"Inlet"});
        let read = |id, samples| -> MeterValuesRequest {
            serde_json::from_value(
                json!({"connectorId":id, "meterValue":[{"timestamp":time,"sampledValue":samples}]}),
            )
            .unwrap()
        };
        let station = read(0, vec![sample.clone()]);
        let connector = read(1, vec![sample.clone()]);
        assert_eq!(
            station_import_kw(&[station.clone(), connector], at),
            Some(12.5)
        );
        assert_eq!(station_import_kw(&[station.clone(), station], at), None);
        for (key, value) in [
            ("phase", "L1"),
            ("location", "Outlet"),
            ("format", "SignedData"),
            ("measurand", "Energy.Active.Import.Register"),
            ("unit", "Wh"),
            ("value", "NaN"),
            ("value", "-1"),
        ] {
            let mut wrong = sample.clone();
            wrong[key] = json!(value);
            assert_eq!(station_import_kw(&[read(0, vec![wrong])], at), None);
        }
    }
}
