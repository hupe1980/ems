//! Local HTTP presentation for the industrial shift.
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use ems_sim::peak::{Evidence, Moment, Reading, Select, Trace, replay};
use std::{fmt::Write as _, sync::Arc};

const fn moment_slug(value: Moment) -> &'static str {
    match value {
        Moment::Early => "early",
        Moment::Delayed => "delayed",
        Moment::Unavoidable => "unavoidable",
        Moment::Closed => "closed",
    }
}

const fn evidence_slug(value: Evidence) -> &'static str {
    match value {
        Evidence::Complete => "complete",
        Evidence::StaleMeter => "stale-meter",
        Evidence::BatteryUnknown => "battery-unknown",
        Evidence::TariffReview => "tariff-review",
        Evidence::LimitedBattery => "limited-battery",
        Evidence::MissingClose => "missing-close",
    }
}

fn render(r: &Reading, q: &Select) -> String {
    let offset = r.at.offset().whole_hours();
    let mut html = format!(
        r#"<!doctype html><html lang="de"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Lastspitze · Schichtsimulation</title><link rel="stylesheet" href="/app.css"><link rel="stylesheet" href="/shift.css"><main class="shift"><p class="cap">Simulierter Logistikstandort · 25. Oktober · {:02}:{:02} (UTC{offset:+})</p><h1 id="entscheidung" tabindex="-1">Batteriestart im Schichtvergleich</h1><p>Bisherige Jahresspitze: {} kW.</p>"#,
        r.at.hour(),
        r.at.minute(),
        r.prior_peak_kw
    );
    if r.minutes_left > 0 {
        let branch = if q.moment == Moment::Early {
            "Jetzt starten"
        } else {
            "Zwei Minuten später"
        };
        let _ = write!(html, "<p>Gewählter Start: <strong>{branch}</strong></p>");
    }
    let evidence = evidence_slug(q.evidence);
    let current = |moment| {
        if q.moment == moment {
            " aria-current=\"step\""
        } else {
            ""
        }
    };
    if let Some(import) = r.import_kwh {
        let _ = write!(html, "<p>Seit Intervallbeginn bezogen: {import} kWh.</p>");
    } else {
        html.push_str("<p>Aktueller Zählerwert fehlt.</p>");
    }
    if let Some(projected) = r.projected_kw {
        let _ = write!(html, "<p>Ohne Eingriff: {projected} kW prognostiziert.");
        if let Some(cost) = r.unassisted_exposure_eur {
            let _ = write!(
                html,
                " Mögliche zusätzliche Jahresleistungskosten: {cost} €."
            );
        }
        html.push_str("</p>");
    }
    if let (Some(need), Some(kwh)) = (r.required_kw, r.required_ac_kwh) {
        let _ = write!(
            html,
            "<p>Für die verbleibenden {} Minuten sind {need} kW nötig ({kwh} kWh AC).</p>",
            r.minutes_left
        );
    } else if q.moment == Moment::Unavoidable
        && let Some(floor) = r.minimum_mean_kw
    {
        let _ = write!(
            html,
            "<p>Der bisherige Bezug ergibt mindestens {floor} kW. Die Jahresspitze ist bereits überschritten.</p>"
        );
    }
    if let (Some(kw), Some(kwh)) = (r.achievable_kw, r.deliverable_ac_kwh) {
        let _ = write!(
            html,
            "<p>Verfügbarer Beitrag: {kw} kW für {} Minuten ({kwh} kWh).</p>",
            r.minutes_left
        );
    }
    if let Some(residual) = r.residual_peak_kw {
        let _ = write!(
            html,
            "<section class=\"forecast\"><h2>{residual} kW mit Batterie</h2><p>Modellierter Viertelstundenmittelwert."
        );
        if let Some(cost) = r.residual_exposure_eur {
            let _ = write!(
                html,
                " Mögliche zusätzliche Jahresleistungskosten: {cost} €."
            );
        }
        html.push_str("</p></section>");
    }
    if r.evidence == Evidence::TariffReview {
        html.push_str("<p>Tarifband offen. Die Kostenprognose wartet auf den Tarif.</p>");
    }
    if r.evidence == Evidence::BatteryUnknown {
        html.push_str("<p>Batteriedaten fehlen. Verfügbare Unterstützung unbekannt.</p>");
    }
    let _ = write!(
        html,
        r##"<nav class="choices" aria-label="Batteriestart"><a{} href="?moment=early&amp;evidence={evidence}#entscheidung">Jetzt starten</a><a{} href="?moment=delayed&amp;evidence={evidence}#entscheidung">Zwei Minuten warten</a></nav>"##,
        current(Moment::Early),
        current(Moment::Delayed)
    );
    if r.revealed {
        if let (Some(mean), Some(kwh), Some(at)) = (r.closed_mean_kw, r.closed_kwh, r.closed_at) {
            let _ = write!(
                html,
                "<h2 id=\"abschluss\" tabindex=\"-1\">{mean} kW im abgeschlossenen Intervall</h2><p>Simulierter Zählerabschluss um {:02}:{:02} (UTC{:+}): {kwh} kWh bei ruhender Batterie.</p>",
                at.hour(),
                at.minute(),
                at.offset().whole_hours()
            );
        } else {
            html.push_str("<h2 id=\"abschluss\" tabindex=\"-1\">Zählerabschluss ausstehend</h2>");
        }
    } else {
        let _ = write!(
            html,
            r##"<p><a href="?moment={}&amp;evidence={evidence}&amp;reveal=true#abschluss">Zählerabschluss aufdecken</a></p>"##,
            moment_slug(q.moment)
        );
    }
    if r.required_kw.is_some_and(|kw| kw > 0.0) && r.achievable_kw.is_some() {
        html.push_str("<details><summary>Letzter möglicher Start</summary>");
        match (r.latest_feasible_delay_min, r.next_minute_mean_kw) {
            (Some(delay), Some(mean)) => {
                let _ = write!(
                    html,
                    "<p>Spätester Start: binnen {delay} Minuten. Eine Minute später bleiben {mean} kW.</p>"
                );
            }
            _ => html.push_str("<p>Mit dieser Batterie reicht keine verbleibende Startminute.</p>"),
        }
        html.push_str("</details>");
    }
    html.push_str(r#"<details><summary>Annahmen prüfen</summary><p>Restlast ohne Eingriff: 900 kW. Batterie im Referenzfall: sofort verfügbar, bis zu 120 kW AC und 10 kWh nutzbar. Der Beispieltarif rechnet mit 100 €/kW pro Jahr.</p></details><details><summary>Weitere Schichtfälle</summary><nav><a href="?moment=unavoidable">Spitze überschritten</a> · <a href="?evidence=stale-meter">Zählerwert fehlt</a> · <a href="?evidence=battery-unknown">Batteriedaten fehlen</a> · <a href="?evidence=tariff-review">Tarifband offen</a> · <a href="?evidence=limited-battery">60-kW-Batterie</a> · <a href="?moment=delayed&amp;evidence=missing-close&amp;reveal=true#abschluss">Zählerabschluss ausstehend</a></nav></details></main></html>"#);
    html
}

pub fn router() -> anyhow::Result<Router> {
    let trace = Arc::new(Trace::bundled()?);
    Ok(Router::new()
        .route(
            "/shift.css",
            get(async || {
                ems_service::edge::file(include_str!("shift.css"), "text/css; charset=utf-8")
            }),
        )
        .route(
            "/",
            get(
                async |State(t): State<Arc<Trace>>, Query(q): Query<Select>| {
                    ems_service::edge::file(render(&replay(&t, &q), &q), "text/html; charset=utf-8")
                },
            ),
        )
        .route(
            "/v1/demo/industrial",
            get(async |State(t): State<Arc<Trace>>, Query(q): Query<Select>| Json(replay(&t, &q))),
        )
        .with_state(trace))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use time::Duration;

    fn fixture() -> Trace {
        Trace::bundled().unwrap()
    }

    fn select(moment: Moment, evidence: Evidence, reveal: bool) -> Select {
        Select {
            moment,
            evidence,
            reveal,
        }
    }

    fn compare_number(value: &Value, expected: i32) {
        assert_eq!(value.as_f64(), Some(f64::from(expected)));
    }

    #[test]
    fn earliest_and_delayed_choices_reveal_the_same_reconciled_close() {
        let trace = fixture();
        for (moment, required, delivered, residual, at) in [
            (Moment::Early, 120, 10, 1100, "2026-10-25T02:55:00+02:00"),
            (Moment::Delayed, 200, 6, 1116, "2026-10-25T02:57:00+02:00"),
        ] {
            let sealed = json!(replay(&trace, &select(moment, Evidence::Complete, false)));
            assert_eq!(sealed["at"], at);
            assert!(sealed["closed_mean_kw"].is_null());
            let opened = json!(replay(&trace, &select(moment, Evidence::Complete, true)));
            for (field, expected) in [
                ("required_kw", required),
                ("deliverable_ac_kwh", delivered),
                ("residual_peak_kw", residual),
                ("closed_mean_kw", 1140),
                ("closed_kwh", 285),
            ] {
                compare_number(&opened[field], expected);
            }
        }
        assert_eq!(
            json!(replay(
                &trace,
                &select(Moment::Closed, Evidence::Complete, false)
            ))["at"],
            "2026-10-25T02:00:00+01:00"
        );
    }

    #[test]
    fn a_future_closed_register_cannot_change_an_earlier_decision() {
        let mut t = fixture();
        let choice = select(Moment::Delayed, Evidence::Complete, false);
        let initial = replay(&t, &choice);
        let sealed = render(&initial, &choice);
        let json = json!(initial);
        let close = t.start + Duration::minutes(15);
        t.register.iter_mut().find(|p| p.at == close).unwrap().wh += 1000;
        t.closed[0].wh += 1000;
        let after = replay(&t, &choice);
        assert_eq!(sealed, render(&after, &choice));
        assert_eq!(json, json!(after));
        let reveal = select(Moment::Delayed, Evidence::Complete, true);
        compare_number(&json!(replay(&t, &reveal))["closed_mean_kw"], 1144);
        t.closed[0].wh += 20;
        assert!(replay(&t, &reveal).closed_mean_kw.is_none());
        t.register
            .iter_mut()
            .find(|p| p.at == t.start + Duration::minutes(12))
            .unwrap()
            .wh += 500;
        assert_ne!(json, json!(replay(&t, &choice)));
    }

    #[test]
    fn last_feasible_minute_respects_the_ac_limit_and_missing_observations() {
        let t = fixture();
        let scenario = |e| json!(replay(&t, &select(Moment::Early, e, false)));
        assert_eq!(scenario(Evidence::Complete)["latest_feasible_delay_min"], 0);
        compare_number(&scenario(Evidence::Complete)["next_minute_mean_kw"], 1108);
        for missing in [
            Evidence::LimitedBattery,
            Evidence::StaleMeter,
            Evidence::BatteryUnknown,
        ] {
            assert!(scenario(missing)["latest_feasible_delay_min"].is_null());
        }
        let pending = select(Moment::Delayed, Evidence::MissingClose, true);
        assert!(render(&replay(&t, &pending), &pending).contains("Zählerabschluss ausstehend"));
        assert!(scenario(Evidence::TariffReview)["residual_exposure_eur"].is_null());
    }
}
