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

/// Whole Wh at one commissioned charging outlet. The transaction and sample
/// time must match the requested boundary. The fixture uses a raw aggregate
/// import register. Wh and kWh must resolve exactly to whole Wh.
#[must_use]
pub fn outlet_register_wh(
    reading: &MeterValuesRequest,
    connector: i32,
    transaction: i32,
    at: DateTime,
) -> Option<i32> {
    if connector <= 0
        || reading.connector_id != connector
        || reading.transaction_id != Some(transaction)
    {
        return None;
    }
    let mut values = reading
        .meter_value
        .iter()
        .filter(|m| m.timestamp == at)
        .flat_map(|m| &m.sampled_value)
        .filter(|s| {
            matches!(
                s.measurand,
                None | Some(Measurand::EnergyActiveImportRegister)
            ) && matches!(s.location, None | Some(Location::Outlet))
                && matches!(s.format, None | Some(ValueFormat::Raw))
                && s.phase.is_none()
        });
    let sample = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let value = sample.value.parse::<ocpp_kit::types::Decimal>().ok()?;
    let wh = match sample.unit {
        None | Some(UnitOfMeasure::Wh) => value,
        Some(UnitOfMeasure::KWh) => value.checked_pow10(3)?,
        _ => return None,
    };
    i32::try_from(i64::try_from(wh).ok()?)
        .ok()
        .filter(|v| *v >= 0)
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
    #[test]
    fn departure_register_requires_one_aligned_transaction_outlet() {
        let at = DateTime::UNIX_EPOCH;
        let read = |samples| {
            serde_json::from_value::<MeterValuesRequest>(json!({
            "connectorId":1,"transactionId":7,"meterValue":[{"timestamp":at,"sampledValue":samples}]
        })).unwrap()
        };
        let sample = json!({"value":"123"});
        let good = read(json!([sample]));
        for (raw, expected) in [
            ("0.123", Some(123)),
            ("0.0001", None),
            ("99999999999999", None),
        ] {
            let scaled = read(json!([{"value":raw,"unit":"kWh"}]));
            assert_eq!(outlet_register_wh(&scaled, 1, 7, at), expected);
        }
        assert_eq!(outlet_register_wh(&good, 1, 7, at), Some(123));
        assert_eq!(outlet_register_wh(&good, 2, 7, at), None);
        assert_eq!(outlet_register_wh(&good, 1, 8, at), None);
        assert_eq!(
            outlet_register_wh(
                &good,
                1,
                7,
                DateTime::parse("2026-01-01T00:00:00Z").unwrap()
            ),
            None
        );
        assert_eq!(
            outlet_register_wh(&read(json!([sample, sample])), 1, 7, at),
            None
        );
        for (key, value) in [
            ("location", "Inlet"),
            ("measurand", "Power.Active.Import"),
            ("phase", "L1"),
            ("format", "SignedData"),
            ("value", "-1"),
            ("value", "1.1"),
            ("unit", "kW"),
        ] {
            let mut bad = sample.clone();
            bad[key] = json!(value);
            assert_eq!(outlet_register_wh(&read(json!([bad])), 1, 7, at), None);
        }
    }
}
