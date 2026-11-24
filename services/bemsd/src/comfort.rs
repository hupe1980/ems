//! One commissioned zone input and the two-station charging budget it can release.
use crate::charging::{BANK_LIMIT_KW, Recording};
use anyhow::{Result, ensure};
use axum::{Json, Router, routing::get};
use bacnet_kit::{
    Device,
    client::{Client, Sample, StatusFlags},
};
use serde::Serialize;
use std::{
    fmt::Write,
    net::{Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};
use tokio::net::UdpSocket;

const RESERVE_KW: i32 = 6;
const MAX_AGE: Duration = Duration::from_secs(30);
const CELSIUS: u32 = 62; // BACnetEngineeringUnits.degrees-celsius

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Condition {
    Comfortable,
    OutsideBand,
    Fault,
    Abnormal,
    Invalid,
    Stale,
    Unavailable,
}

impl Condition {
    fn evaluate(sample: Option<&Sample>, now: Instant) -> Self {
        let Some(s) = sample else {
            return Self::Unavailable;
        };
        if s.flags.fault {
            Self::Fault
        } else if s.flags != StatusFlags::default() {
            Self::Abnormal
        } else if !s.value.is_finite() || s.unit != CELSIUS || s.observed_at > now {
            Self::Invalid
        } else if s.is_stale(now, MAX_AGE) {
            Self::Stale
        } else if (20.0..=23.0).contains(&s.value) {
            Self::Comfortable
        } else {
            Self::OutsideBand
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Comfortable => "Temperatur im Komfortband",
            Self::OutsideBand => "Temperatur außerhalb des Komfortbands",
            Self::Fault => "Sensor meldet Störung",
            Self::Abnormal => "Sensorstatus nicht normal",
            Self::Invalid => "Messwert ungültig",
            Self::Stale => "Letzter Messwert abgelaufen",
            Self::Unavailable => "Keine gültige Messung",
        }
    }
}

#[derive(Clone, Serialize)]
struct Decision {
    condition: Condition,
    reported_c: Option<f32>,
    last_good_c: Option<f32>,
    last_good_age_s: Option<u64>,
    reserve_held_kw: i32,
    charging_limit_kw: i32,
    measured_bank_kw: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    feeder_comparisons: Vec<crate::feeder::Comparison>,
}

async fn decide(
    sample: Option<Sample>,
    last_good: &mut Option<Sample>,
    now: Instant,
) -> Result<Decision> {
    let reported_c = sample
        .as_ref()
        .filter(|s| s.unit == CELSIUS && s.value.is_finite())
        .map(|s| s.value);
    let mut condition = Condition::evaluate(sample.as_ref(), now);
    if matches!(condition, Condition::Comfortable | Condition::OutsideBand) {
        *last_good = sample;
    } else if condition == Condition::Unavailable
        && last_good.as_ref().is_some_and(|s| s.is_stale(now, MAX_AGE))
    {
        condition = Condition::Stale;
    }
    let reserve = if condition == Condition::Comfortable {
        0
    } else {
        RESERVE_KW
    };
    let limit = BANK_LIMIT_KW - reserve;
    let (deadline, measured_bank_kw) = Recording::measure_bank(limit).await?;
    let feeder_comparisons = if condition == Condition::Fault {
        crate::feeder::rehearse(deadline.timestamp().as_second(), measured_bank_kw).await?
    } else {
        Vec::new()
    };
    Ok(Decision {
        condition,
        reported_c,
        last_good_c: last_good.as_ref().map(|s| s.value),
        last_good_age_s: last_good
            .as_ref()
            .map(|s| now.duration_since(s.observed_at).as_secs()),
        reserve_held_kw: reserve,
        charging_limit_kw: limit,
        measured_bank_kw,
        feeder_comparisons,
    })
}

// Poll exactly the commissioned point. The reference device can be silenced
// without closing its socket, so the last observation ages through a real timeout.
async fn read_point(socket: &UdpSocket, device: Option<&Device>) -> Result<Option<Sample>> {
    let SocketAddr::V4(peer) = socket.local_addr()? else {
        anyhow::bail!("IPv4 point required")
    };
    ensure!(
        peer.ip().is_loopback(),
        "rehearsal requires a loopback point"
    );
    let mut reader = tokio::task::spawn_blocking(move || {
        let mut client = Client::bind(Ipv4Addr::LOCALHOST, peer, Duration::from_millis(500))?;
        Ok::<_, bacnet_kit::client::Error>(client.sample(1).ok())
    });
    let mut request = [0; 2048];
    let mut response = [0; 2048];
    loop {
        tokio::select! {
            result = &mut reader => return Ok(result??),
            result = socket.recv_from(&mut request) => {
                let (size, source) = result?;
                if let Some(device) = device
                    && let Ok(Some(n)) = device.respond(&request[..size], &mut response)
                {
                    socket.send_to(&response[..n], source).await?;
                }
            }
        }
    }
}

async fn rehearse() -> Result<Vec<Decision>> {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let mut device = Device::new(37, 0, 21.5)?;
    let mut last_good = None;
    let good = read_point(&socket, Some(&device)).await?;
    let start = Instant::now();
    let first = decide(good, &mut last_good, start).await?;
    device.set_fault(true);
    let fault = read_point(&socket, Some(&device)).await?;
    let second = decide(fault, &mut last_good, start + Duration::from_secs(10)).await?;
    let silent = read_point(&socket, None).await?;
    let third = decide(silent, &mut last_good, start + MAX_AGE).await?;
    Ok(vec![first, second, third])
}

pub(super) async fn router() -> Result<Router> {
    let evidence = rehearse().await?;
    let mut html = String::from(
        r#"<!doctype html><html lang="de"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Heizreserve · Westgebäude</title><link rel="stylesheet" href="/app.css"><link rel="stylesheet" href="/replay.css"><main class="screen building"><a href="/">Zur Gebäudeübergabe</a><p class="cap">Westgebäude · Simulation</p><h1>Heizreserve</h1><p>24 kW stehen für Laden und Heizreserve zur Verfügung. Ein gültiger Zonenwert von 20 bis 23 °C gibt die reservierten 6 kW zum Laden frei.</p>"#,
    );
    for row in &evidence {
        write!(
            html,
            "<section class=\"reserve-decision\"><h2>{}</h2><p><strong>{} kW Ladegrenze</strong> · {} kW Heizreserve</p><meter min=\"0\" max=\"{BANK_LIMIT_KW}\" value=\"{}\" aria-label=\"Freigegebene Ladeleistung in Kilowatt\"></meter><dl><dt>OCPP · beide Ladestationen gemessen</dt><dd>{}</dd><dt>BACnet · gemeldeter Zonenwert</dt><dd>{}</dd><dt>Letzter gültiger Zonenwert</dt><dd>{}</dd></dl>",
            row.condition.label(),
            row.charging_limit_kw,
            row.reserve_held_kw,
            row.charging_limit_kw,
            row.measured_bank_kw
                .map_or_else(|| "Unbekannt".into(), |kw| format!("{kw} kW")),
            row.reported_c.map_or_else(
                || "Kein gültiger Wert".into(),
                |c| format!("{} °C", c.to_string().replace('.', ","))
            ),
            row.last_good_c.zip(row.last_good_age_s).map_or_else(
                || "Unbekannt".into(),
                |(c, age)| format!("{} °C · vor {age} s", c.to_string().replace('.', ","))
            )
        )?;
        if !row.feeder_comparisons.is_empty() {
            html.push_str("<details><summary>KNX-Zähler am Ladeabgang vergleichen</summary><p>Drei mögliche Leistungsmessungen bei derselben Stationssumme. Zulässig sind 2 s Empfangsalter und 0,1 kW Abweichung.</p><ul>");
            for comparison in &row.feeder_comparisons {
                write!(html, "<li>{}</li>", comparison.description())?;
            }
            html.push_str("</ul></details>");
        }
        html.push_str("</section>");
    }
    html.push_str("</main></html>");
    Ok(Router::new()
        .route("/comfort", get(async move || axum::response::Html(html)))
        .route(
            "/v1/demo/building/comfort",
            get(async move || Json(evidence)),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_fault_and_silent_point_hold_the_reserve_on_both_station_meters() {
        let rows = tokio::time::timeout(Duration::from_secs(10), rehearse())
            .await
            .expect("rehearsal exceeded its deadline")
            .unwrap();
        let comparisons = &rows[1].feeder_comparisons;
        assert_eq!(comparisons.len(), 3);
        let evidence = serde_json::to_value(comparisons).unwrap();
        assert_eq!(evidence[0]["delta_kw"], 0.0);
        assert_eq!(evidence[1]["delta_kw"], 4.0);
        assert!(evidence[2]["delta_kw"].is_null());
        for (r, (condition, kw, age)) in rows.iter().zip([
            (Condition::Comfortable, 24, 0),
            (Condition::Fault, 18, 10),
            (Condition::Stale, 18, 30),
        ]) {
            assert_eq!(r.condition, condition);
            assert_eq!(r.charging_limit_kw, kw);
            assert_eq!(r.measured_bank_kw, Some(f64::from(kw)));
            assert_eq!(r.last_good_c, Some(21.5));
            assert_eq!(r.last_good_age_s, Some(age));
        }
    }

    #[test]
    fn only_recent_credible_celsius_in_band_releases_the_reserve() {
        let now = Instant::now();
        let mut s = Sample {
            value: 20.0,
            unit: CELSIUS,
            flags: StatusFlags::default(),
            observed_at: now,
        };
        assert_eq!(Condition::evaluate(None, now), Condition::Unavailable);
        assert_eq!(
            Condition::evaluate(Some(&s), now + MAX_AGE),
            Condition::Stale
        );
        assert_eq!(
            Condition::evaluate(Some(&s), now + MAX_AGE - Duration::from_nanos(1)),
            Condition::Comfortable
        );
        for (value, expected) in [
            (23.0, Condition::Comfortable),
            (19.9, Condition::OutsideBand),
            (23.1, Condition::OutsideBand),
            (f32::NAN, Condition::Invalid),
        ] {
            s.value = value;
            assert_eq!(Condition::evaluate(Some(&s), now), expected);
        }
        s.value = 21.5;
        s.unit = 64;
        assert_eq!(Condition::evaluate(Some(&s), now), Condition::Invalid);
        s.unit = CELSIUS;
        for (bit, expected) in [
            (128, Condition::Abnormal),
            (64, Condition::Fault),
            (32, Condition::Abnormal),
            (16, Condition::Abnormal),
        ] {
            s.flags = StatusFlags::from_bits(bit);
            assert_eq!(Condition::evaluate(Some(&s), now), expected);
        }
    }
}
