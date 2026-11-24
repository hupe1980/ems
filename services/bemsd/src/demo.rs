//! Synthetic, read-only handover mounted exclusively on the loopback demo.
use crate::charging::{Outcome, Recording, Station, Timeline};
use axum::extract::State;
use ems_drv::ocpp::DateTime;
use std::{fmt::Write as _, sync::Arc};

use axum::{Json, Router, extract::Query, http::StatusCode, response::IntoResponse, routing::get};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Default, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum Case {
    #[default]
    Confirmed,
    NoSolar,
    Unconfirmed,
    AboveRequest,
    IdleStation,
    DelayedArrival,
    StationRejoin,
    StaleCharger,
    Stale,
    OverLimit,
    UnknownPv,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum Moment {
    Requested,
    Observed,
    Deadline,
    Rejoined,
    Buffered,
    Restored,
    #[default]
    Handover,
}

impl Moment {
    fn at(self, timeline: &Timeline) -> DateTime {
        match self {
            Self::Requested => timeline.requested_at,
            Self::Observed => timeline.first_sample_at,
            Self::Deadline => timeline.deadline_at,
            Self::Rejoined => timeline.rejoined_at,
            Self::Buffered => timeline.buffered_at,
            Self::Restored => timeline.restored_at,
            Self::Handover => timeline.handover_at,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Requested => "Anforderung",
            Self::Observed => "Erste Messung",
            Self::Deadline => "Stichtag",
            Self::Rejoined => "Wieder verbunden",
            Self::Buffered => "Gepufferte Messung",
            Self::Restored => "Neue Messung",
            Self::Handover => "Übergabe",
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Selection {
    case: Case,
    moment: Moment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Delivery {
    Pending,
    WithinLimit,
    AboveLimit,
    Unconfirmed,
}

impl Delivery {
    fn assess(requested_kw: i32, final_kw: Option<f64>) -> Self {
        match final_kw {
            Some(kw) if kw > f64::from(requested_kw) => Self::AboveLimit,
            Some(_) => Self::WithinLimit,
            None => Self::Unconfirmed,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Pending => "Prüfung ausstehend",
            Self::WithinLimit => "Ladegrenze eingehalten",
            Self::AboveLimit => "Ladegrenze überschritten",
            Self::Unconfirmed => "Lieferung unbestätigt",
        }
    }
}

#[derive(Serialize)]
struct Handover {
    simulated: bool,
    charging_source: &'static str,
    #[serde(serialize_with = "DateTime::serialize")]
    at: DateTime,
    moment: Moment,
    limit_kw: i32,
    net_import_kw: Option<i32>,
    last_grid_reading_kw: Option<i32>,
    grid_read_at: Option<&'static str>,
    pv_kw: Option<i32>,
    heating_kw: Option<i32>,
    charging_kw: Option<i32>,
    headroom_kw: Option<i32>,
    unattributed_kw: Option<i32>,
    #[serde(serialize_with = "DateTime::serialize")]
    request_at: DateTime,
    requested_kw: i32,
    #[serde(serialize_with = "Option::<DateTime>::serialize")]
    first_sample_at: Option<DateTime>,
    first_sample_kw: Option<f64>,
    #[serde(serialize_with = "DateTime::serialize")]
    final_sample_due_at: DateTime,
    #[serde(serialize_with = "Option::<DateTime>::serialize")]
    final_sample_at: Option<DateTime>,
    final_sample_kw: Option<f64>,
    stations: [Station; 2],
    delivery: Delivery,
}

impl Handover {
    fn replay(case: Case, moment: Moment, recording: &Recording) -> Self {
        let limit_kw = 60;
        let at = moment.at(&recording.timeline);
        let snapshot = moment == Moment::Handover;
        let net_import_kw = match case {
            _ if !snapshot => None,
            Case::Stale => None,
            Case::OverLimit => Some(63),
            _ => Some(52),
        };
        let pv_kw = match case {
            _ if !snapshot => None,
            Case::UnknownPv => None,
            Case::NoSolar => Some(0),
            _ => Some(12),
        };
        let final_sample_due_at = recording.timeline.deadline_at;
        let requested_kw = crate::charging::BANK_LIMIT_KW;
        let stations = recording.stations(
            match case {
                Case::AboveRequest => Outcome::Above,
                Case::StationRejoin => Outcome::Rejoin,
                Case::IdleStation => Outcome::Zero,
                Case::DelayedArrival => Outcome::Delayed,
                Case::Unconfirmed => Outcome::Missing,
                Case::StaleCharger => Outcome::Stale,
                _ => Outcome::Confirmed,
            },
            at,
        );
        let first_sample_kw: Option<f64> = stations.iter().map(|station| station.initial_kw).sum();
        let final_sample_kw = stations
            .iter()
            .map(|station| station.final_kw)
            .sum::<Option<f64>>();
        let heating_kw = 20;
        let charging_kw = 24;
        let final_sample_at = final_sample_kw.and(stations[0].final_measured_at);
        Self {
            simulated: true,
            charging_source: "ocpp-1.6j-station-telemetry",
            at,
            moment,
            limit_kw,
            net_import_kw,
            last_grid_reading_kw: snapshot.then(|| net_import_kw.unwrap_or(51)),
            grid_read_at: snapshot.then_some(if net_import_kw.is_some() {
                "09:15:00"
            } else {
                "09:10:00"
            }),
            pv_kw,
            heating_kw: snapshot.then_some(heating_kw),
            charging_kw: snapshot.then_some(charging_kw),
            headroom_kw: net_import_kw.map(|kw| limit_kw - kw),
            unattributed_kw: net_import_kw
                .zip(pv_kw)
                .map(|(grid, pv)| grid + pv - heating_kw - charging_kw),
            request_at: recording.timeline.requested_at,
            requested_kw,
            first_sample_at: first_sample_kw.map(|_| recording.timeline.first_sample_at),
            first_sample_kw,
            final_sample_due_at,
            final_sample_at,
            final_sample_kw,
            stations,
            delivery: if at < final_sample_due_at {
                Delivery::Pending
            } else {
                Delivery::assess(requested_kw, final_sample_kw)
            },
        }
    }
}

fn power(value: Option<i32>) -> String {
    value.map_or_else(|| "Unbekannt".into(), |kw| format!("{kw} kW"))
}

fn local_time(at: DateTime) -> String {
    // The bundled November incident is in Berlin standard time (UTC+1).
    let seconds = (at.timestamp().as_second() + 3600).rem_euclid(86400);
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

fn replay_link(case: Case, moment: Moment) -> String {
    use clap::ValueEnum as _;
    format!(
        "?case={}&amp;moment={}#zeitpunkt",
        case.to_possible_value().unwrap().get_name(),
        moment.to_possible_value().unwrap().get_name()
    )
}

impl Case {
    fn moments(self) -> &'static [Moment] {
        use clap::ValueEnum as _;
        match self {
            Self::StationRejoin => Moment::value_variants(),
            _ => &[
                Moment::Requested,
                Moment::Observed,
                Moment::Deadline,
                Moment::Handover,
            ],
        }
    }
}

fn timeline_nav(html: &mut String, selection: &Selection) {
    let moments = selection.case.moments();
    let index = moments.iter().position(|m| *m == selection.moment).unwrap();
    let _ = write!(
        html,
        r#"<nav aria-label="Zeitverlauf"><p>Schritt {} von {}</p>"#,
        index + 1,
        moments.len()
    );
    for (target, direction) in [
        (index.checked_sub(1).and_then(|i| moments.get(i)), "Zurück"),
        (moments.get(index + 1), "Weiter"),
    ] {
        if let Some(m) = target {
            let _ = write!(
                html,
                r#"<a href="{}">{direction}: {}</a> "#,
                replay_link(selection.case, *m),
                m.label()
            );
        }
    }
    html.push_str("</nav>");
}

async fn page(
    State(recording): State<Arc<Recording>>,
    Query(selection): Query<Selection>,
) -> impl IntoResponse {
    if !selection.case.moments().contains(&selection.moment) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let h = Handover::replay(selection.case, selection.moment, &recording);
    let mut html = format!(
        r#"<!doctype html><html lang="de"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{} · Westgebäude</title><link rel="stylesheet" href="/app.css"><link rel="stylesheet" href="/replay.css"><main class="screen building"><p class="cap">Simuliertes Gebäude · 22. November 2026</p><h1>Westgebäude</h1><h2 id="zeitpunkt" tabindex="-1">{} · {} MEZ</h2><p><strong>{}</strong></p>"#,
        h.moment.label(),
        h.moment.label(),
        local_time(h.at),
        h.delivery.label()
    );
    let _ = write!(
        html,
        "<p>Ladegrenze {} kW · angefordert um {} MEZ wegen der Anschlussgrenze.</p>",
        h.requested_kw,
        local_time(h.request_at)
    );
    match (
        h.final_sample_kw.zip(h.final_sample_at),
        h.first_sample_kw,
        h.delivery,
    ) {
        (Some((kw, measured)), _, _) => {
            let _ = write!(
                html,
                "<p>{kw} kW gemessen um {} MEZ.</p>",
                local_time(measured)
            );
        }
        (None, first, delivery) => {
            if let Some((kw, time)) = first.zip(h.first_sample_at) {
                let _ = write!(
                    html,
                    "<p>Erste Messung um {} MEZ: {kw} kW.</p>",
                    local_time(time)
                );
            }
            let detail = if delivery == Delivery::Pending {
                "Messung erwartet"
            } else {
                "Zeitgleiche Stationsmessung fehlt"
            };
            let _ = write!(
                html,
                "<p>{detail} · Stichtag {} MEZ.</p>",
                local_time(h.final_sample_due_at)
            );
        }
    }
    if h.final_sample_kw.is_some()
        && let Some(arrival) = h.stations.iter().filter_map(|s| s.final_received_at).max()
        && h.final_sample_at.is_some_and(|measured| arrival > measured)
    {
        let _ = write!(
            html,
            "<p>Nachträglich bestätigt · letzter Eingang {} MEZ.</p>",
            local_time(arrival)
        );
    }
    if matches!(selection.case, Case::StationRejoin) {
        let station = &h.stations[1];
        let message = if !station.connected {
            "West-02 ist getrennt. Die letzte Messung bleibt im Verlauf."
        } else if station.final_kw.is_some() {
            "Die aktuelle Messung von West-02 zählt wieder zur Ladegruppe."
        } else if station.buffered_received_at.is_some() {
            "West-02 hat eine ältere Messung nachgeliefert. Die aktuelle Leistung ist offen."
        } else if h.at >= recording.timeline.rejoined_at {
            "West-02 ist wieder verbunden. Eine aktuelle Leistungsmessung steht aus."
        } else {
            "Beide Ladestationen sind verbunden."
        };
        let _ = write!(html, "<p>{message}</p>");
    }
    html.push_str(r#"<p class="cap">Quelle: OCPP-Ladestationstelemetrie</p>"#);
    timeline_nav(&mut html, &selection);
    station_evidence(&mut html, &h);
    if selection.moment == Moment::Handover {
        snapshot(&mut html, &h);
    }
    case_selector(&mut html, selection.moment);
    html.push_str(
        r#"<p><a href="/comfort">Heizreserve und Ladeleistung vergleichen</a></p></main></html>"#,
    );
    ems_service::edge::file(html, "text/html; charset=utf-8").into_response()
}

fn station_evidence(html: &mut String, h: &Handover) {
    html.push_str("<details><summary>Nachweis je Ladestation</summary><dl>");
    for station in &h.stations {
        let profile = match station.profile_accepted {
            None => "Antwort ausstehend",
            Some(true) => "angenommen",
            Some(false) => "nicht angenommen",
        };
        let observed = if let (Some(kw), Some(measured), Some(received)) = (
            station.final_kw,
            station.final_measured_at,
            station.final_received_at,
        ) {
            format!(
                "{kw} kW gemessen um {} MEZ · Eingang {} MEZ",
                local_time(measured),
                local_time(received)
            )
        } else if !station.connected
            && let Some((kw, measured)) = station.initial_kw.zip(h.first_sample_at)
        {
            format!(
                "Verbindung getrennt · letzter Messwert {kw} kW um {} MEZ",
                local_time(measured)
            )
        } else if h.delivery == Delivery::Pending {
            station.initial_kw.map_or_else(
                || "Messung ausstehend".into(),
                |kw| format!("Erste Messung: {kw} kW"),
            )
        } else {
            "Kein zeitgleicher Messwert".into()
        };
        let mut observed = observed;
        if let Some(received) = station.buffered_received_at
            && let Some(measured) = h.first_sample_at
        {
            let _ = write!(
                observed,
                " · frühere Messung um {} MEZ, nachgeliefert um {} MEZ",
                local_time(measured),
                local_time(received)
            );
        }
        let _ = write!(
            html,
            "<dt>{}</dt><dd>Angefordertes Profil {profile} · {observed}</dd>",
            station.id
        );
    }
    html.push_str("</dl></details>");
}
fn snapshot(html: &mut String, h: &Handover) {
    let headline = match h.headroom_kw {
        None => "Netzmessung veraltet".to_owned(),
        Some(kw) if kw < 0 => format!("{} kW über Anschlussgrenze", -kw),
        Some(kw) => format!("{kw} kW Anschlussreserve"),
    };
    let _ = write!(html, r#"<h2>{headline}</h2><dl class="group">"#);
    if let (None, Some(kw), Some(at)) = (h.net_import_kw, h.last_grid_reading_kw, h.grid_read_at) {
        let _ = write!(
            html,
            r#"<div class="row"><dt>Letzter Netzbezug um {at} MEZ</dt><dd>{kw} kW</dd></div>"#
        );
    }
    for (name, value) in [
        ("Anschlussgrenze", Some(h.limit_kw)),
        ("Netzbezug", h.net_import_kw),
        ("PV-Erzeugung", h.pv_kw),
        ("Heizung", h.heating_kw),
        ("Ladegruppe", h.charging_kw),
        ("Nicht zugeordneter Verbrauch", h.unattributed_kw),
    ] {
        let _ = write!(
            html,
            r#"<div class="row"><dt>{name}</dt><dd>{}</dd></div>"#,
            power(value)
        );
    }
    html.push_str("</dl>");
}

fn case_selector(html: &mut String, moment: Moment) {
    html.push_str(r#"<details><summary>Beispielfall wechseln</summary><nav><ul>"#);
    for (case, label) in [
        (Case::StationRejoin, "West-02 nach Verbindungsabbruch"),
        (Case::DelayedArrival, "Verspäteter Eingang"),
        (Case::Confirmed, "Bestätigt"),
        (Case::AboveRequest, "Ladegrenze überschritten"),
        (Case::IdleStation, "West-02 misst 0 kW"),
        (Case::Unconfirmed, "Bestätigung fehlt"),
        (Case::StaleCharger, "Stationsmessung veraltet"),
        (Case::Stale, "Netzmessung veraltet"),
        (Case::OverLimit, "Grenze überschritten"),
        (Case::NoSolar, "Ohne PV-Erzeugung"),
        (Case::UnknownPv, "PV-Messung fehlt"),
    ] {
        let _ = write!(
            html,
            r#"<li><a href="{}">{label}</a></li>"#,
            replay_link(
                case,
                if case.moments().contains(&moment) {
                    moment
                } else {
                    Moment::Deadline
                }
            )
        );
    }
    html.push_str("</ul></nav></details>");
}
pub(super) fn router(recording: Recording) -> Router {
    Router::new()
        .route(
            "/replay.css",
            get(async || {
                ems_service::edge::file(include_str!("replay.css"), "text/css; charset=utf-8")
            }),
        )
        .route("/", get(page))
        .route(
            "/v1/demo/building",
            get(
                async |State(recording): State<Arc<Recording>>, Query(q): Query<Selection>| {
                    if !q.case.moments().contains(&q.moment) {
                        return StatusCode::BAD_REQUEST.into_response();
                    }
                    Json(Handover::replay(q.case, q.moment, &recording)).into_response()
                },
            ),
        )
        .with_state(Arc::new(recording))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn verdict_uses_the_incident_sample_even_when_the_present_bank_is_at_limit() {
        for (sample, expected) in [
            (Some(20.0), Delivery::WithinLimit),
            (Some(24.0), Delivery::WithinLimit),
            (Some(26.0), Delivery::AboveLimit),
            (None, Delivery::Unconfirmed),
        ] {
            assert_eq!(Delivery::assess(24, sample), expected);
        }
        let trace = Recording::load().await.unwrap();
        for (case, expected) in [
            (Case::AboveRequest, Delivery::AboveLimit),
            (Case::Unconfirmed, Delivery::Unconfirmed),
            (Case::StaleCharger, Delivery::Unconfirmed),
            (Case::Stale, Delivery::WithinLimit),
        ] {
            let record = Handover::replay(case, Moment::Handover, &trace);
            assert_eq!(record.charging_kw, Some(24));
            assert_eq!(record.delivery, expected);
            assert_eq!(
                record.final_sample_at.is_some(),
                record.final_sample_kw.is_some()
            );
        }
    }

    #[tokio::test]
    async fn rewind_uses_only_the_station_evidence_received_by_the_cursor() {
        let trace = Recording::load().await.unwrap();
        let view = |moment| Handover::replay(Case::DelayedArrival, moment, &trace);
        let request = view(Moment::Requested);
        assert_eq!(request.delivery, Delivery::Pending);
        assert!(
            request
                .stations
                .iter()
                .all(|s| s.profile_accepted.is_none())
        );
        assert_eq!(
            (request.first_sample_at, request.first_sample_kw),
            (None, None)
        );
        let first = view(Moment::Observed);
        assert_eq!(first.first_sample_kw, Some(28.0));
        assert!(
            first
                .stations
                .iter()
                .all(|s| s.profile_accepted == Some(true))
        );
        let deadline = view(Moment::Deadline);
        assert_eq!(deadline.delivery, Delivery::Unconfirmed);
        assert_eq!(
            (deadline.stations[0].final_kw, deadline.stations[1].final_kw),
            (Some(12.0), None)
        );
        assert_eq!(deadline.last_grid_reading_kw, None);
        let handover = view(Moment::Handover);
        assert_eq!(handover.final_sample_kw, Some(24.0));
        assert_eq!(handover.final_sample_at, Some(trace.timeline.deadline_at));
        assert!(handover.stations[1].final_received_at.unwrap() > trace.timeline.deadline_at);
        assert_eq!(
            serde_json::to_value(deadline).unwrap(),
            serde_json::to_value(view(Moment::Deadline)).unwrap()
        );

        // Changing a future message must not change any returned field at the deadline.
        let mut data: serde_json::Value =
            serde_json::from_str(include_str!("../examples/charging-bank.json")).unwrap();
        data["west_02"]["demand_at_deadline"]["payload"][0]["meterValue"][0]["sampledValue"][0]["value"] =
            "2".into();
        let changed = Recording::simulate(&data.to_string()).await.unwrap();
        let altered = |moment| Handover::replay(Case::DelayedArrival, moment, &changed);
        assert_eq!(
            serde_json::to_value(view(Moment::Deadline)).unwrap(),
            serde_json::to_value(altered(Moment::Deadline)).unwrap()
        );
        assert_eq!(altered(Moment::Handover).final_sample_kw, Some(14.0));
    }

    #[tokio::test]
    async fn rejoined_station_needs_fresh_measurement_before_counting_again() {
        let mut data: serde_json::Value =
            serde_json::from_str(include_str!("../examples/charging-bank.json")).unwrap();
        for (watts, expected) in [("14000", 26.0), ("0", 12.0)] {
            data["west_02"]["after_rejoin"]["payload"][0]["meterValue"][0]["sampledValue"][0]["value"] =
                watts.into();
            let recording = Recording::simulate(&data.to_string()).await.unwrap();
            let view = |moment| Handover::replay(Case::StationRejoin, moment, &recording);
            for moment in [Moment::Deadline, Moment::Rejoined, Moment::Buffered] {
                let handover = view(moment);
                assert!(handover.stations[0].connected);
                assert_eq!(handover.stations[0].final_kw, Some(12.0));
                assert_eq!(handover.stations[1].connected, moment != Moment::Deadline);
                assert_eq!(
                    handover.stations[1].buffered_received_at.is_some(),
                    moment == Moment::Buffered
                );
                assert_eq!(handover.final_sample_kw, None);
            }
            let fresh = view(Moment::Restored);
            assert_eq!(fresh.final_sample_kw, Some(expected));
            assert_eq!(fresh.final_sample_at, Some(recording.timeline.restored_at));
            assert_eq!(view(Moment::Deadline).stations[1].final_received_at, None);
        }
        data["west_02"]["after_rejoin"]["payload"][0]["meterValue"][0]["timestamp"] =
            data["timeline"]["first_sample_at"].clone();
        let recording = Recording::simulate(&data.to_string()).await.unwrap();
        assert_eq!(
            Handover::replay(Case::StationRejoin, Moment::Restored, &recording).delivery,
            Delivery::Unconfirmed
        );
    }

    #[tokio::test]
    async fn removing_one_station_observation_makes_a_confirmed_delivery_unknown() {
        let mut data: serde_json::Value =
            serde_json::from_str(include_str!("../examples/charging-bank.json")).unwrap();
        let at_deadline =
            |record: &Recording| Handover::replay(Case::Confirmed, Moment::Deadline, record);
        let baseline = Recording::load().await.unwrap();
        assert_eq!(at_deadline(&baseline).delivery, Delivery::WithinLimit);
        assert_eq!(
            Handover::replay(Case::IdleStation, Moment::Deadline, &baseline).stations[1].final_kw,
            Some(0.0)
        );
        // Only connector-level telemetry remains on the wire.
        data["west_02"]["demand_at_deadline"]["payload"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        let changed = Recording::simulate(&data.to_string()).await.unwrap();
        let missing = at_deadline(&changed);
        assert_eq!(
            (missing.delivery, missing.final_sample_kw),
            (Delivery::Unconfirmed, None)
        );
        assert_eq!(missing.stations[1].profile_accepted, Some(true));
    }

    #[tokio::test]
    async fn pv_and_missing_evidence_remain_distinct() {
        let trace = Recording::load().await.unwrap();
        let h = Handover::replay(Case::Unconfirmed, Moment::Handover, &trace);
        assert_eq!(
            (h.headroom_kw, h.unattributed_kw, h.final_sample_kw),
            (Some(8), Some(20), None)
        );
        assert_eq!(
            Handover::replay(Case::UnknownPv, Moment::Handover, &trace).unattributed_kw,
            None
        );
        assert_eq!(
            Handover::replay(Case::Stale, Moment::Handover, &trace).headroom_kw,
            None
        );
        assert_eq!(
            Handover::replay(Case::OverLimit, Moment::Handover, &trace).headroom_kw,
            Some(-3)
        );
    }
}
