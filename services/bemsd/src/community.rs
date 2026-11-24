//! Synthetic quarter-hour allocation for one locally selected identity.
mod reveal;
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use ems_core::prelude::Slot;
use ems_grid::sharing::{Aufteilung, Community, Member, allocate_by};
use reveal::Reveal;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, macros::datetime};
use uuid::Uuid;

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Viewer {
    MemberA,
    MemberB,
    Coordinator,
    FormerMember,
}

impl Viewer {
    fn name(self) -> &'static str {
        match self {
            Self::MemberA => "a",
            Self::MemberB => "b",
            Self::Coordinator => "koordination",
            Self::FormerMember => "ehemalig",
        }
    }
}

#[derive(Clone)]
struct Session {
    viewer: Viewer,
    auth: String,
}

pub fn router(viewer: Viewer) -> Router {
    let password = Uuid::new_v4();
    let name = viewer.name();
    eprintln!("Local demo login: {name} / {password}");
    let auth = format!("Basic {}", STANDARD.encode(format!("{name}:{password}")));
    Router::new()
        .route("/", get(page))
        .route("/v1/demo/community", get(api))
        .with_state(Session { viewer, auth })
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Case {
    #[default]
    Static,
    Cascading,
    Missing,
    Estimated,
    Zero,
    Late,
    Corrected,
    Duplicate,
    Outage,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Selection {
    case: Case,
    interval: Option<u8>,
    member: Option<String>,
    reveal: Option<Reveal>,
}

#[derive(Serialize)]
struct Reading {
    period: &'static str,
    key: &'static str,
    status: &'static str,
    rows: Vec<Row>,
    #[serde(skip_serializing_if = "Option::is_none")]
    coordinator: Option<Summary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_complete: Option<Box<Reading>>,
}

#[derive(Serialize)]
struct Row {
    member: &'static str,
    consumed_kwh: Option<String>,
    shared_kwh: Option<String>,
}

#[derive(Serialize)]
struct Summary {
    measured: usize,
    expected: usize,
    supply_kwh: String,
    residual_kwh: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_residual_kwh: Option<String>,
}

const PERIODS: [(&str, OffsetDateTime); 2] = [
    (
        "25.10.2026 · 02:00–02:15 +02:00",
        datetime!(2026-10-25 00:00 UTC),
    ),
    (
        "25.10.2026 · 02:00–02:15 +01:00",
        datetime!(2026-10-25 01:00 UTC),
    ),
];

fn demands(case: Case, second: bool) -> Vec<Option<Decimal>> {
    let mut values = vec![Some(Decimal::TWO)];
    if second {
        // Revisions replace a single member/interval cell. Estimated inputs never close it.
        let revisions: &[Option<u8>] = match case {
            Case::Missing | Case::Estimated | Case::Outage => &[None],
            Case::Late => &[None, Some(8)],
            Case::Corrected => &[Some(8), Some(4)],
            Case::Duplicate => &[Some(8), Some(8)],
            Case::Zero => &[Some(0)],
            _ => &[Some(8)],
        };
        let mut cell = None;
        for revision in revisions {
            cell = revision.map(Decimal::from);
        }
        values.push(cell);
    }
    values
}

fn read(viewer: Viewer, selection: &Selection) -> Result<Reading, StatusCode> {
    let interval = selection.interval.unwrap_or(2);
    let index = usize::from(interval.checked_sub(1).ok_or(StatusCode::NOT_FOUND)?);
    let &(period, start) = PERIODS.get(index).ok_or(StatusCode::NOT_FOUND)?;
    // B joined between the two quarters. A former member has no current grant.
    let names = if index == 0 {
        &["a"][..]
    } else {
        &["a", "b"][..]
    };
    let coordinator = matches!(viewer, Viewer::Coordinator);
    let member = selection
        .member
        .as_deref()
        .or_else(|| (!coordinator).then(|| viewer.name()));
    if member.is_some_and(|m| !names.contains(&m) || (!coordinator && m != viewer.name())) {
        return Err(StatusCode::NOT_FOUND);
    }
    let case = selection.case;
    if selection.reveal.is_some()
        && !(coordinator && index == 1 && member.is_none() && matches!(case, Case::Static))
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let mut values = demands(case, index == 1);
    if let Some(reveal) = selection.reveal {
        reveal.project(&mut values);
    }
    let dynamic = matches!(case, Case::Cascading);
    let community = Community::new(
        "Demo",
        names
            .iter()
            .map(|&n| Member::new(n, Decimal::ONE))
            .collect(),
    );
    let supply = Decimal::TEN;
    let allocation = values
        .iter()
        .copied()
        .collect::<Option<Vec<_>>>()
        .map(|demand| {
            allocate_by(
                &community,
                Slot::containing(start),
                supply,
                &demand,
                if dynamic {
                    Aufteilung::Dynamisch
                } else {
                    Aufteilung::Statisch
                },
            )
        })
        .transpose()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let rows = names
        .iter()
        .enumerate()
        .filter(|(_, name)| member.is_none_or(|m| m == **name))
        .map(|(i, &name)| Row {
            member: name,
            consumed_kwh: values[i].map(|v| v.normalize().to_string()),
            shared_kwh: allocation
                .as_ref()
                .map(|a| a.shares[i].shared.normalize().to_string()),
        })
        .collect();
    let status = if allocation.is_some() {
        "Vollständig"
    } else {
        match case {
            Case::Estimated => "Schätzung · Messung ausstehend",
            Case::Outage => "Verbindung unterbrochen",
            _ => "Messung fehlt",
        }
    };
    let last_complete = if index == 1
        && matches!(case, Case::Outage)
        && !matches!(viewer, Viewer::MemberB)
        && member != Some("b")
    {
        Some(Box::new(read(
            viewer,
            &Selection {
                interval: Some(1),
                member: selection.member.clone(),
                ..Selection::default()
            },
        )?))
    } else {
        None
    };
    Ok(Reading {
        period,
        key: if dynamic { "Kaskadierend" } else { "Statisch" },
        status,
        rows,
        last_complete,
        coordinator: coordinator.then(|| Summary {
            measured: values.iter().flatten().count(),
            expected: names.len(),
            supply_kwh: supply.to_string(),
            minimum_residual_kwh: selection
                .reveal
                .map(|_| reveal::minimum(&values, supply).normalize().to_string()),
            residual_kwh: allocation.map(|a| a.unallocated.normalize().to_string()),
        }),
    })
}

fn response(session: &Session, headers: &HeaderMap, selection: &Selection, html: bool) -> Response {
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        != Some(session.auth.as_str())
    {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Basic realm=\"EMS-Demo\"")],
        )
            .into_response();
    }
    let mut result = match read(session.viewer, selection) {
        Ok(v) if html => ems_service::edge::file(
            render(&v, selection, session.viewer),
            "text/html; charset=utf-8",
        )
        .into_response(),
        Ok(v) => Json(v).into_response(),
        Err(status) => status.into_response(),
    };
    result
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    result
}

async fn api(State(s): State<Session>, h: HeaderMap, Query(q): Query<Selection>) -> Response {
    response(&s, &h, &q, false)
}
async fn page(State(s): State<Session>, h: HeaderMap, Query(q): Query<Selection>) -> Response {
    response(&s, &h, &q, true)
}

fn render(v: &Reading, selection: &Selection, viewer: Viewer) -> String {
    let role = if v.coordinator.is_some() {
        "Koordination"
    } else {
        "Mitglied"
    };
    let mut text = format!(
        r#"<!doctype html><html lang="de"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/app.css"><title>Gemeinschaftsstrom · {role}</title><main><p class="cap">Simulierte Gemeinschaft · {role}</p><h1>Gemeinschaftsstrom</h1><h2 id="intervall" tabindex="-1">{}</h2><p>{}</p>"#,
        v.period, v.status
    );
    if let Some(s) = &v.coordinator {
        text.push_str(&format!(
            "<section aria-label=\"Bilanz\"><h2>Restmenge</h2><strong>{}</strong><p>{}/{} Messungen · Erzeugung {} kWh</p></section>",
            s.residual_kwh.as_ref().map_or_else(|| "ausstehend".into(), |v| format!("{v} kWh")), s.measured, s.expected, s.supply_kwh
        ));
    }
    if let Some(snapshot) = selection.reveal {
        text.push_str(&snapshot.panel(v));
    } else if matches!(viewer, Viewer::Coordinator) && selection.member.is_none() {
        text.push_str(r#"<p><a href="?interval=2&amp;reveal=none#messfrage">Welche Messung entscheidet?</a></p>"#);
    }
    for row in &v.rows {
        let heading = if v.coordinator.is_some() {
            format!("Mitglied {}", row.member.to_ascii_uppercase())
        } else {
            "Ihr Anteil".into()
        };
        let assigned = row.shared_kwh.as_ref().map_or_else(
            || "Zuteilung ausstehend".to_owned(),
            |v| format!("{v} kWh zugeteilt"),
        );
        let consumed = row.consumed_kwh.as_ref().map_or_else(
            || "Verbrauchsmessung ausstehend".to_owned(),
            |v| format!("{v} kWh verbraucht"),
        );
        text.push_str(&format!(
            "<section aria-label=\"{heading}\"><h2>{heading}</h2><strong>{assigned}</strong><p>{consumed}</p></section>"
        ));
    }
    text.push_str(&format!(
        "<details><summary>Aufteilungsschlüssel</summary><p>{} · gleiche Gewichte</p></details>",
        v.key
    ));
    // Only member identifiers checked against the selected quarter reach the links.
    let member = selection
        .member
        .as_deref()
        .map_or(String::new(), |id| format!("&amp;member={id}"));
    if let Some(last) = &v.last_complete {
        text.push_str(&format!(
            "<p>Letztes vollständiges Intervall: <a href=\"?interval=1{member}#intervall\">{}</a></p>",
            last.period
        ));
    }
    let interval = selection.interval.unwrap_or(2);
    let case = serde_json::to_value(selection.case).expect("a case serializes as a name");
    text.push_str("<nav aria-label=\"Viertelstunde\">");
    for (number, (period, _)) in [1, 2].into_iter().zip(PERIODS) {
        if number == 1
            && (matches!(viewer, Viewer::MemberB) || selection.member.as_deref() == Some("b"))
        {
            continue;
        }
        let current = if number == interval {
            " aria-current=\"page\""
        } else {
            ""
        };
        text.push_str(&format!(
            "<a{current} href=\"?interval={number}&amp;case={}{member}#intervall\">{period}</a> ",
            case.as_str().unwrap_or("static")
        ));
    }
    text.push_str("</nav>");
    text.push_str("<nav aria-label=\"Messfälle\">");
    for (value, label) in [
        ("static", "Statisch"),
        ("cascading", "Kaskadierend"),
        ("missing", "Messlücke"),
        ("estimated", "Schätzung"),
        ("late", "Nachlieferung"),
        ("corrected", "Korrektur"),
        ("duplicate", "Duplikat"),
        ("zero", "Nullverbrauch"),
        ("outage", "Ausfall"),
    ] {
        let current = if case.as_str() == Some(value) {
            " aria-current=\"page\""
        } else {
            ""
        };
        text.push_str(&format!(
            "<a{current} href=\"?case={value}&amp;interval={interval}{member}#intervall\">{label}</a> "
        ));
    }
    text.push_str("</nav></main></html>");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_and_meter_coverage_determine_the_allocation() {
        for (case, shares, residual) in [
            (Case::Static, ["2", "5"], "3"),
            (Case::Cascading, ["2", "8"], "0"),
            (Case::Duplicate, ["2", "5"], "3"),
            (Case::Late, ["2", "5"], "3"),
            (Case::Corrected, ["2", "4"], "4"),
            (Case::Zero, ["2", "0"], "8"),
        ] {
            let v = read(
                Viewer::Coordinator,
                &Selection {
                    case,
                    ..Selection::default()
                },
            )
            .unwrap();
            assert_eq!(
                v.rows
                    .iter()
                    .map(|r| r.shared_kwh.as_deref().unwrap())
                    .collect::<Vec<_>>(),
                shares
            );
            assert_eq!(
                v.coordinator.unwrap().residual_kwh.as_deref(),
                Some(residual)
            );
        }
        for case in [Case::Missing, Case::Estimated, Case::Outage] {
            let v = read(
                Viewer::Coordinator,
                &Selection {
                    case,
                    ..Selection::default()
                },
            )
            .unwrap();
            assert!(v.rows.iter().all(|r| r.shared_kwh.is_none()));
            assert_eq!(v.coordinator.unwrap().measured, 1);
            if matches!(case, Case::Outage) {
                let last = v.last_complete.unwrap();
                assert_ne!(last.period, v.period);
                assert_eq!(last.status, "Vollständig");
            }
        }
    }

    #[test]
    fn only_the_coordinator_can_reveal_measured_cells_in_the_selected_quarter() {
        for (stage, bound, measured) in [
            (Reveal::None, "0", 0),
            (Reveal::A, "3", 1),
            (Reveal::B, "0", 1),
            (Reveal::Both, "3", 2),
        ] {
            let selection = Selection {
                reveal: Some(stage),
                ..Selection::default()
            };
            let v = read(Viewer::Coordinator, &selection).unwrap();
            let c = v.coordinator.unwrap();
            assert_eq!(c.minimum_residual_kwh.as_deref(), Some(bound));
            assert_eq!(c.measured, measured);
            assert_eq!(c.residual_kwh.is_some(), measured == 2);
            for viewer in [Viewer::MemberA, Viewer::MemberB, Viewer::FormerMember] {
                for html in [false, true] {
                    let session = Session {
                        viewer,
                        auth: "Basic local".into(),
                    };
                    let mut h = HeaderMap::new();
                    assert_eq!(
                        response(&session, &h, &selection, html).status(),
                        StatusCode::UNAUTHORIZED
                    );
                    h.insert(header::AUTHORIZATION, session.auth.parse().unwrap());
                    assert_eq!(
                        response(&session, &h, &selection, html).status(),
                        StatusCode::NOT_FOUND
                    );
                }
            }
        }
        for selection in [
            Selection {
                reveal: Some(Reveal::A),
                interval: Some(1),
                ..Selection::default()
            },
            Selection {
                case: Case::Cascading,
                reveal: Some(Reveal::Both),
                ..Selection::default()
            },
        ] {
            assert!(matches!(
                read(Viewer::Coordinator, &selection),
                Err(StatusCode::NOT_FOUND)
            ));
        }
        assert_eq!(
            reveal::minimum(&[Some(Decimal::ZERO), None], Decimal::TEN),
            Decimal::from(5)
        );
    }

    #[test]
    fn reads_require_both_current_and_interval_access() {
        let s = Session {
            viewer: Viewer::MemberB,
            auth: "Basic demo".into(),
        };
        let mut h = HeaderMap::new();
        for html in [false, true] {
            assert_eq!(
                response(&s, &h, &Selection::default(), html).status(),
                StatusCode::UNAUTHORIZED
            );
        }
        h.insert(header::AUTHORIZATION, s.auth.parse().unwrap());
        for q in [
            Selection {
                member: Some("a".into()),
                ..Selection::default()
            },
            Selection {
                interval: Some(1),
                ..Selection::default()
            },
        ] {
            assert_eq!(response(&s, &h, &q, true).status(), StatusCode::NOT_FOUND);
            assert_eq!(response(&s, &h, &q, false).status(), StatusCode::NOT_FOUND);
        }
        let row = read(s.viewer, &Selection::default()).unwrap();
        assert!(row.coordinator.is_none());
        assert_eq!(row.rows.len(), 1);
        assert_eq!(row.rows[0].member, "b");
        assert!(read(Viewer::FormerMember, &Selection::default()).is_err());
    }
}
