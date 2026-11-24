//! Independent meter at the EV feeder. The ETS binding assigns DPT 14.056, import positive.
use anyhow::Result;
use knx::routing::RoutingEvent;
use serde::Serialize;
use std::{net::Ipv4Addr, time::Duration};
use tokio::{net::UdpSocket, time::timeout};

const GROUP: u16 = (2 << 11) | (1 << 8) | 3;
const SOURCE: u16 = 0x1125; // 1.1.37
const MAX_AGE_S: i64 = 2;
const TOLERANCE_W: f64 = 100.0;

fn feeder_w(datagram: &[u8]) -> Option<f64> {
    let RoutingEvent::Indication(telegram) = RoutingEvent::parse(datagram).ok()? else {
        return None;
    };
    if telegram.source != SOURCE || u16::from(telegram.destination) != GROUP {
        return None;
    }
    knx::decode_power_watts(telegram.indicated_value()?.as_bytes())
        .ok()
        .map(f64::from)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    Agrees,
    Higher,
    Lower,
    Unavailable,
}

#[derive(Clone, Serialize)]
pub(super) struct Comparison {
    age_s: Option<i64>,
    feeder_kw: Option<f64>,
    delta_kw: Option<f64>,
    verdict: Verdict,
}

impl Comparison {
    fn at(deadline: i64, bank_kw: Option<f64>, observation: Option<(i64, f64)>) -> Self {
        let age_s = observation.and_then(|(at, _)| deadline.checked_sub(at));
        let watts = observation
            .filter(|_| age_s.is_some_and(|age| (0..=MAX_AGE_S).contains(&age)))
            .map(|(_, watts)| watts);
        let delta = watts.zip(bank_kw).map(|(w, kw)| w - kw * 1000.0);
        let verdict = match delta {
            None => Verdict::Unavailable,
            Some(w) if w.abs() <= TOLERANCE_W => Verdict::Agrees,
            Some(w) if w > 0.0 => Verdict::Higher,
            Some(_) => Verdict::Lower,
        };
        Self {
            age_s,
            feeder_kw: watts.map(|w| w / 1000.0),
            delta_kw: delta.map(|w| w / 1000.0),
            verdict,
        }
    }

    pub(super) fn description(&self) -> String {
        match (self.verdict, self.feeder_kw, self.delta_kw) {
            (Verdict::Agrees, Some(kw), _) => {
                format!("{kw} kW · stimmt mit der Stationssumme überein")
            }
            (Verdict::Higher, Some(kw), Some(delta)) => {
                format!("{kw} kW · {delta} kW über der Stationssumme")
            }
            (Verdict::Lower, Some(kw), Some(delta)) => {
                format!("{kw} kW · {} kW unter der Stationssumme", -delta)
            }
            _ if self.age_s.is_some_and(|age| age > MAX_AGE_S) => format!(
                "Kein Vergleich · Empfang vor {} s",
                self.age_s.unwrap_or_default()
            ),
            _ => "Kein Vergleich · Messnachweis fehlt".into(),
        }
    }
}

// Reference sender's KNXnet/IP routing indication, cEMI L_Data.ind, GroupValueWrite.
// The receiver uses the upstream protocol parser independently of this fixed wire image.
fn indication(watts: f32) -> [u8; 21] {
    let mut packet = [
        6, 16, 5, 0x30, 0, 21, 0x29, 0, 0xbc, 0xe0, 0x11, 0x25, 0x11, 0x03, 5, 0, 0x80, 0, 0, 0, 0,
    ];
    packet[17..].copy_from_slice(&watts.to_be_bytes());
    packet
}

pub(super) async fn rehearse(deadline: i64, bank_kw: Option<f64>) -> Result<Vec<Comparison>> {
    let mut cases = Vec::new();
    // Each alternative has its own logical receive tick relative to the OCPP deadline.
    for (watts, received_at) in [
        (18_000.0, deadline),
        (22_000.0, deadline - 1),
        (18_000.0, deadline - 3),
    ] {
        // Isolate alternatives so a delayed datagram cannot become the next case.
        let meter = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let gateway = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        gateway.connect(meter.local_addr()?).await?;
        meter.connect(gateway.local_addr()?).await?;
        meter.send(&indication(watts)).await?;
        let mut datagram = [0; 512];
        let observed = match timeout(Duration::from_millis(500), gateway.recv(&mut datagram)).await
        {
            Ok(size) => feeder_w(&datagram[..size?]).map(|w| (received_at, w)),
            Err(_) => None,
        };
        cases.push(Comparison::at(deadline, bank_kw, observed));
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_commissioned_indicated_power_qualifies() {
        let good = indication(-1000.0);
        assert_eq!(feeder_w(&good), Some(-1000.0));
        for (offset, replacement) in [(11, 0x26), (13, 4), (6, 0x11), (16, 0), (5, 20)] {
            let mut packet = good;
            packet[offset] = replacement;
            assert_eq!(feeder_w(&packet), None);
        }
        assert_eq!(feeder_w(&indication(f32::NAN)), None);
        assert_eq!(feeder_w(&good[..17]), None);
    }

    #[test]
    fn agreement_needs_two_time_aligned_measurements() {
        let compare = |time, w| Comparison::at(10, Some(18.0), Some((time, w)));
        for watts in [17_900.0, 18_100.0] {
            assert_eq!(compare(8, watts).verdict, Verdict::Agrees);
        }
        assert_eq!(compare(8, 17_899.0).verdict, Verdict::Lower);
        assert_eq!(compare(8, 18_101.0).verdict, Verdict::Higher);
        assert_eq!(compare(10, -1000.0).delta_kw, Some(-19.0));
        for time in [7, 11] {
            assert_eq!(compare(time, 18_000.0).delta_kw, None);
        }
        let absent = Comparison::at(10, None, Some((10, 0.0)));
        assert_eq!(absent.feeder_kw, Some(0.0));
        assert_eq!(absent.verdict, Verdict::Unavailable);
    }
}
