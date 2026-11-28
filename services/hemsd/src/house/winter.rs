//! Close a winter electrical day from the same meter before and after the morning appliances.
use super::grid::Grid;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::fmt::Write as _;
use time::{Duration, OffsetDateTime};

#[derive(Serialize)]
struct Day {
    morning: super::thermal::Arm,
    opening_soc: f64,
    closing_soc: f64,
    boundaries: Vec<Boundary>,
}

#[derive(Serialize)]
struct Boundary {
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
    import_wh: f64,
    export_wh: f64,
}

async fn arm(scenario: &crate::Scenario, factor: f64, target: u32, goal: i32) -> Result<Day> {
    let start = super::thermal::START;
    let mut grid = Grid::configured(scenario.start(), scenario).await?;
    let mut sample = grid.sample().await?;
    let initial_soc = grid.battery_soc().await?;
    while sample.at < start {
        sample = grid.quarter(0).await?;
    }
    ensure!(sample.at == start, "thermal window must open on a quarter");
    let morning = Box::pin(super::thermal::arm_on(
        factor,
        target,
        false,
        false,
        Some(goal),
        &mut grid,
        sample,
    ))
    .await?;
    close(grid, initial_soc, morning).await
}

async fn close(mut grid: Grid, opening_soc: f64, morning: super::thermal::Arm) -> Result<Day> {
    let start = grid.boundaries.first().context("missing day opening")?.at;
    let end = start + Duration::days(1);
    while grid.boundaries.last().context("missing register")?.at < end {
        grid.quarter(0).await?;
    }
    let closing_soc = grid.battery_soc().await?;
    ensure!(
        grid.boundaries.len() == 97,
        "winter day needs 97 register boundaries"
    );
    let mut boundaries: Vec<Boundary> = Vec::with_capacity(97);
    for sample in &grid.boundaries {
        let boundary = Boundary {
            at: sample.at,
            import_wh: sample.energy_in.context("missing import register")?.get(),
            export_wh: sample.energy_out.context("missing export register")?.get(),
        };
        if let Some(last) = boundaries.last() {
            ensure!(
                boundary.at - last.at == Duration::minutes(15)
                    && boundary.import_wh >= last.import_wh
                    && boundary.export_wh >= last.export_wh,
                "incomplete winter meter interval"
            );
        }
        boundaries.push(boundary);
    }
    Ok(Day {
        morning,
        opening_soc,
        closing_soc,
        boundaries,
    })
}

/// Electrical meter evidence for both heating requests on the Berlin winter fixture.
#[derive(Serialize)]
pub struct Comparison {
    fixture: &'static str,
    morning_fixture_sha256: String,
    target_wh: u32,
    managed: Day,
    baseline: Day,
}

/// Exercise 96 electrical quarters and a morning water and EV service window.
///
/// # Errors
/// Returns an error for missing device evidence or a stalled protocol exchange.
pub async fn trial(target_wh: u32, goal: i32) -> Result<Comparison> {
    let scenario = crate::Scenario::winter_with_grid_event(crate::HouseholdConfig::default());
    let morning_fixture_sha256 = super::thermal::fixture_digest(target_wh, Some(goal))?;
    let (managed, baseline) = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        tokio::try_join!(
            Box::pin(arm(&scenario, 1.0, target_wh, goal)),
            Box::pin(arm(&scenario, 0.5, target_wh, goal))
        )
    })
    .await
    .context("winter day trial timed out")??;
    Ok(Comparison {
        fixture: "accelerated-berlin-winter-v1",
        morning_fixture_sha256,
        target_wh,
        managed,
        baseline,
    })
}

impl Day {
    fn imported_wh(&self) -> f64 {
        self.boundaries[self.boundaries.len() - 1].import_wh - self.boundaries[0].import_wh
    }
}

impl std::fmt::Display for Comparison {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Berlin winter day · 15 January 2026 · accelerated simulation"
        )?;
        writeln!(f, "{}", self.service_summary())?;
        for (name, day) in [("Managed", &self.managed), ("Baseline", &self.baseline)] {
            let open = day.boundaries.first().ok_or(std::fmt::Error)?;
            let close = day.boundaries.last().ok_or(std::fmt::Error)?;
            writeln!(
                f,
                "{name} · {} quarters · {:.0} Wh imported · {:.0} Wh exported",
                day.boundaries.len() - 1,
                close.import_wh - open.import_wh,
                close.export_wh - open.export_wh
            )?;
            writeln!(
                f,
                "  Battery charge {:.1}% → {:.1}%",
                day.opening_soc * 100.0,
                day.closing_soc * 100.0
            )?;
            writeln!(f, "  {}", day.morning)?;
        }
        write!(
            f,
            "Thermal observations cover 06:00–06:15 UTC. The outlet closes at 07:00 UTC."
        )
    }
}

impl Comparison {
    fn service_summary(&self) -> String {
        let difference = self.managed.imported_wh() - self.baseline.imported_wh();
        if self.managed.morning.met_service()
            && difference > 0.0
            && let Some(short) = self.baseline.morning.closing_shortfall_wh
            && short > crate::scenario::service::WATER_TOLERANCE_WH
        {
            return format!(
                "The baseline imported {difference:.0} Wh less. At 06:15 UTC its tank was {short:.0} Wh below target."
            );
        }
        if self.managed.morning.met_service() && self.baseline.morning.met_service() {
            return "Both heater settings met the morning room and hot-water targets.".into();
        }
        "The morning readings do not establish equivalent thermal service.".into()
    }

    /// An offline report with drill-down to each metered quarter.
    pub fn html(&self) -> String {
        let mut html = String::from(include_str!("../../ui/thermal-header.html"));
        let _ = write!(
            html,
            "<p><strong>{}</strong></p><p>15 January 2026 · Room target 20–22 °C and hot-water target {} Wh at 06:15 UTC. Electrical readings cover the full day.</p>",
            self.service_summary(),
            self.target_wh
        );
        for (name, day) in [("Managed", &self.managed), ("Baseline", &self.baseline)] {
            let open = &day.boundaries[0];
            let close = &day.boundaries[day.boundaries.len() - 1];
            let _ = write!(
                html,
                "<section><h2>{name}</h2><p><strong>{:.0} Wh imported</strong> · {:.0} Wh exported over 24 simulated hours</p><p>Battery charge · {:.1}% → {:.1}%</p>",
                close.import_wh - open.import_wh,
                close.export_wh - open.export_wh,
                day.opening_soc * 100.0,
                day.closing_soc * 100.0
            );
            day.morning.write_html(&mut html);
            html.push_str("<details><summary>96 metered quarters</summary><table><caption>Energy per quarter, Berlin local time</caption><thead><tr><th>Starting at</th><th>Import Wh</th><th>Export Wh</th></tr></thead><tbody>");
            for pair in day.boundaries.windows(2) {
                let at = ems_core::calendar::to_berlin(pair[0].at);
                let _ = write!(
                    html,
                    "<tr><th scope=row>{:02}:{:02}</th><td>{:.0}</td><td>{:.0}</td></tr>",
                    at.hour(),
                    at.minute(),
                    pair[1].import_wh - pair[0].import_wh,
                    pair[1].export_wh - pair[0].export_wh
                );
            }
            html.push_str("</tbody></table></details></section>");
        }
        html.push_str("</main></html>");
        html
    }
}
