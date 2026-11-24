//! Standalone review of the same ordered quarters emitted by the JSON formatter.

use anyhow::Result;
use ems_core::calendar::{FIRST_YEAR, LAST_YEAR, to_berlin};
use ems_sim::peak::{IntervalStatus, Report};
use std::fmt::Write;
use time::format_description::well_known::Rfc3339;

pub(super) fn render(report: &Report, tolerance_wh: u64) -> Result<String> {
    let mut html = include_str!("review.html").to_owned();
    let mut exceptions = report
        .intervals()
        .iter()
        .enumerate()
        .filter(|(_, quarter)| quarter.status != IntervalStatus::WithinTolerance)
        .map(|(index, _)| index)
        .peekable();

    write!(
        html,
        "<header><h1>Interval review</h1><p class=summary><strong>{} of {}</strong> {} within tolerance</p><p>{} compared · Tolerance {} Wh</p>",
        report.matching_intervals,
        report.interval_count(),
        if report.interval_count() == 1 {
            "quarter"
        } else {
            "quarters"
        },
        report.compared_intervals,
        tolerance_wh
    )?;
    if let Some(first) = exceptions.peek() {
        write!(html, "<a href='#q{first}'>Go to first exception</a>")?;
    }
    html.push_str("</header><main>");
    for (index, quarter) in report.intervals().iter().enumerate() {
        let status = match quarter.status {
            IntervalStatus::RegisterDecreased => "Register decreased",
            IntervalStatus::MissingBoundary => "Missing register boundary",
            IntervalStatus::MissingClosedInterval => "Missing independent close",
            IntervalStatus::OutsideTolerance => "Outside tolerance",
            IntervalStatus::WithinTolerance => "Within tolerance",
        };
        let local = if (FIRST_YEAR..=LAST_YEAR).contains(&quarter.start.year()) {
            to_berlin(quarter.start)
        } else {
            quarter.start
        };
        let utc = quarter.start.format(&Rfc3339)?;
        let offset = local.offset();
        let offset_minutes = offset.whole_minutes().unsigned_abs();
        let sign = if offset.is_negative() { '-' } else { '+' };
        let exception = exceptions.next_if_eq(&index).is_some();
        let class = if exception { "exception" } else { "match" };
        write!(
            html,
            "<section class='{class}'><h2 tabindex='-1' id='q{index}'><time datetime='{utc}'>{} {:02}:{:02} {sign}{:02}:{:02}</time> · {status}</h2><p class=utc>Interval start · <span>{utc}</span></p><table><caption>Meter evidence in Wh</caption>",
            local.date(),
            local.hour(),
            local.minute(),
            offset_minutes / 60,
            offset_minutes % 60
        )?;
        for (name, value) in [
            (
                "Register increase",
                quarter.register_wh.map(|v| v.to_string()),
            ),
            (
                "Independent close",
                quarter.closed_wh.map(|v| v.to_string()),
            ),
            (
                "Difference (register − close)",
                quarter.difference_wh.map(|v| {
                    if v > 0 {
                        format!("+{v}")
                    } else {
                        v.to_string()
                    }
                }),
            ),
        ] {
            write!(
                html,
                "<tr><th scope='row'>{name}</th><td>{}</td></tr>",
                value.as_deref().unwrap_or("Unavailable")
            )?;
        }
        html.push_str("</table>");
        if exception {
            write!(html, "<p>Allowed difference · {tolerance_wh} Wh</p>")?;
        }
        if exception && let Some(next) = exceptions.peek() {
            write!(html, "<a href='#q{next}'>Next exception</a>")?;
        }
        html.push_str("</section>");
    }
    html.push_str("</main></body></html>");
    Ok(html)
}
