//! The closed days the box keeps, and what they add up to.
//!
//! A receipt is gone at midnight unless something keeps it. The box keeps the
//! last [`KEPT_DAYS`] closed days — one small document each — so a household can
//! add its saving up, and take the rows with it as CSV.
//!
//! # Integers, so the month is the sum of its days
//!
//! Money is stored in **whole cents** and energy in **watt-hours**, rounded once
//! when the day closes. A month total is then a sum of integers: it equals the
//! thirty receipts to the cent by construction, where thirty `f64` euros summed
//! and rounded afterwards need not.
//!
//! # A day the box did not close is absent
//!
//! Days are returned sparsely, each with its date. A day the box was off has no
//! row — never a row of zeroes, which would read as a day that cost nothing.
//! The same holds within a day: a box on a wall meters and does not model, so a
//! day it closed carries [`ClosedDay::money`] only where the counterfactual was
//! computed (a replay); a metered day has its energy and its § 14a record and
//! says nothing about euros.

use serde::{Deserialize, Serialize};
use time::Date;

use crate::home::Receipt;

/// How many closed days the box keeps: M11's ninety consecutive day reports.
pub const KEPT_DAYS: usize = 90;

/// The priced half of a closed day, in whole cents.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct Money {
    /// What the household paid for electricity.
    pub bill_ct: i64,
    /// Battery wear, priced.
    pub wear_ct: i64,
    /// Comfort given up, priced.
    pub comfort_ct: i64,
    /// Everything the day cost: bill, wear and comfort among it.
    pub cost_ct: i64,
    /// The same house with the same hardware and no energy manager.
    pub without_ct: i64,
    /// `without_ct − cost_ct`, exactly.
    pub saved_ct: i64,
    /// The part of the saving that reached the bill.
    pub on_bill_ct: i64,
}

/// One closed day, as the box keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClosedDay {
    /// The local day, `YYYY-MM-DD`.
    pub day: String,
    /// What the day cost and saved, where it was priced.
    pub money: Option<Money>,
    /// Bought from the grid, Wh.
    pub imported_wh: i64,
    /// Made by the roof, Wh.
    pub produced_wh: i64,
    /// Minutes under a § 14a reduction, where the day measured them.
    pub reduced_minutes: Option<i64>,
    /// How many reductions the network operator sent.
    pub reductions: u32,
    /// Whether every reduction was respected.
    pub reduction_respected: bool,
    /// Whether the feed-in ceiling was respected.
    pub feed_in_respected: bool,
}

/// Euros to whole cents, half away from zero.
#[allow(clippy::cast_possible_truncation)]
fn cents(eur: f64) -> i64 {
    (eur * 100.0).round() as i64
}

/// Kilowatt-hours to whole watt-hours.
#[allow(clippy::cast_possible_truncation)]
fn watt_hours(kwh: f64) -> i64 {
    (kwh * 1000.0).round() as i64
}

impl ClosedDay {
    /// The day a receipt describes. `priced` says whether its euros are a
    /// computed counterfactual or merely absent — a metered day's receipt
    /// carries zeroes where nothing was modelled, and those must not be kept
    /// as a day that cost nothing.
    #[must_use]
    pub fn of(receipt: &Receipt, priced: bool) -> Self {
        let money = priced.then(|| {
            let cost_ct = cents(receipt.cost_eur);
            let without_ct = cents(receipt.without_eur);
            Money {
                bill_ct: cents(receipt.bill_eur),
                wear_ct: cents(receipt.wear_eur),
                comfort_ct: cents(receipt.comfort_eur),
                cost_ct,
                without_ct,
                // Derived from the two rounded figures beside it, so the
                // receipt's own subtraction holds in cents.
                saved_ct: without_ct - cost_ct,
                on_bill_ct: cents(receipt.on_bill_eur),
            }
        });
        Self {
            day: receipt.day.clone(),
            money,
            imported_wh: watt_hours(receipt.imported_kwh),
            produced_wh: watt_hours(receipt.produced_kwh),
            reduced_minutes: receipt.reduced_minutes,
            reductions: u32::try_from(receipt.reductions).unwrap_or(u32::MAX),
            reduction_respected: receipt.reduction_respected,
            feed_in_respected: receipt.feed_in_respected,
        }
    }

    /// A day the box itself closed, from the report it sends the fleet.
    #[must_use]
    pub fn of_report(day: &ems_core::report::DayKpis) -> Self {
        Self::of(&crate::home::receipt_of(day), day.economics.is_some())
    }

    /// The calendar date, where [`ClosedDay::day`] is one.
    #[must_use]
    pub fn date(&self) -> Option<Date> {
        parse_day(&self.day)
    }
}

/// `YYYY-MM-DD` as a date.
#[must_use]
pub fn parse_day(text: &str) -> Option<Date> {
    let format = time::macros::format_description!("[year]-[month]-[day]");
    Date::parse(text, &format).ok()
}

/// What a run of closed days adds up to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, schemars::JsonSchema)]
pub struct Total {
    /// How many closed days are in the sum.
    pub days: u32,
    /// How many of them were priced — the days [`Total::money`] is a sum of.
    pub priced_days: u32,
    /// The priced days' euros, summed in cents.
    pub money: Money,
    /// Bought from the grid, Wh.
    pub imported_wh: i64,
    /// Made by the roof, Wh.
    pub produced_wh: i64,
    /// How many reductions the network operator sent.
    pub reductions: u32,
    /// Whether every reduction of every day was respected.
    pub reduction_respected: bool,
}

/// The document `GET /v1/days` answers with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct Days {
    /// The closed days in the window, oldest first; a missing date is a day
    /// the box did not close.
    pub days: Vec<ClosedDay>,
    /// Their sum.
    pub total: Total,
}

impl Days {
    /// The days and their sum.
    #[must_use]
    pub fn of(days: Vec<ClosedDay>) -> Self {
        let mut total = Total {
            reduction_respected: true,
            ..Total::default()
        };
        for day in &days {
            total.days += 1;
            total.imported_wh += day.imported_wh;
            total.produced_wh += day.produced_wh;
            total.reductions += day.reductions;
            total.reduction_respected &= day.reduction_respected;
            if let Some(m) = day.money {
                total.priced_days += 1;
                total.money.bill_ct += m.bill_ct;
                total.money.wear_ct += m.wear_ct;
                total.money.comfort_ct += m.comfort_ct;
                total.money.cost_ct += m.cost_ct;
                total.money.without_ct += m.without_ct;
                total.money.saved_ct += m.saved_ct;
                total.money.on_bill_ct += m.on_bill_ct;
            }
        }
        Self { days, total }
    }

    /// Only the days from `from` to `to`, both inclusive where given.
    #[must_use]
    pub fn within(days: Vec<ClosedDay>, from: Option<Date>, to: Option<Date>) -> Self {
        Self::of(
            days.into_iter()
                .filter(|d| {
                    d.date().is_some_and(|date| {
                        from.is_none_or(|f| date >= f) && to.is_none_or(|t| date <= t)
                    })
                })
                .collect(),
        )
    }
}

/// The columns of the CSV, in order.
///
/// Money in euros with a decimal point and two places, energy in kWh with
/// three; an unpriced day leaves its euro columns empty. Every field is a date,
/// a number or `true`/`false` — there is no free text, so nothing a spreadsheet
/// could read as a formula.
pub const CSV_HEADER: &str = "day,bill_eur,wear_eur,comfort_eur,cost_eur,without_hems_eur,saved_eur,\
saved_on_bill_eur,imported_kwh,produced_kwh,reduced_minutes,reductions,reduction_respected,\
feed_in_respected";

/// Whole hundredths or thousandths as a decimal, exactly.
fn decimal(value: i64, places: u32) -> String {
    let scale = 10_i64.pow(places);
    let sign = if value < 0 { "-" } else { "" };
    let abs = value.unsigned_abs();
    let scale_u = scale.unsigned_abs();
    format!(
        "{sign}{}.{:0width$}",
        abs / scale_u,
        abs % scale_u,
        width = places as usize
    )
}

/// The closed days as CSV: a header and one row per day.
#[must_use]
pub fn csv(days: &[ClosedDay]) -> String {
    let mut out = String::from(CSV_HEADER);
    out.push('\n');
    for d in days {
        // A stored day whose date does not parse is not written as text.
        let Some(date) = d.date() else { continue };
        let eur = |pick: fn(&Money) -> i64| {
            d.money
                .as_ref()
                .map_or_else(String::new, |m| decimal(pick(m), 2))
        };
        let row = [
            date.to_string(),
            eur(|m| m.bill_ct),
            eur(|m| m.wear_ct),
            eur(|m| m.comfort_ct),
            eur(|m| m.cost_ct),
            eur(|m| m.without_ct),
            eur(|m| m.saved_ct),
            eur(|m| m.on_bill_ct),
            decimal(d.imported_wh, 3),
            decimal(d.produced_wh, 3),
            d.reduced_minutes
                .map_or_else(String::new, |m| m.to_string()),
            d.reductions.to_string(),
            d.reduction_respected.to_string(),
            d.feed_in_respected.to_string(),
        ];
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

/// The window a request asks for: `?from=YYYY-MM-DD&to=YYYY-MM-DD`, both
/// inclusive and both optional.
#[derive(Debug, Default, Deserialize)]
pub struct Window {
    /// The first day.
    pub from: Option<String>,
    /// The last day.
    pub to: Option<String>,
}

impl Window {
    /// The days of `kept` this window asks for, or `None` where a bound is not
    /// a date or the window runs backwards.
    #[must_use]
    pub fn of(&self, kept: Vec<ClosedDay>) -> Option<Days> {
        let bound = |text: &Option<String>| match text {
            None => Some(None),
            Some(text) => parse_day(text).map(Some),
        };
        let (from, to) = (bound(&self.from)?, bound(&self.to)?);
        if let (Some(from), Some(to)) = (from, to)
            && to < from
        {
            return None;
        }
        Some(Days::within(kept, from, to))
    }
}

/// The CSV as a download the browser neither caches nor sniffs.
#[must_use]
pub fn csv_response(days: &[ClosedDay]) -> axum::response::Response {
    use axum::http::header;
    use axum::response::IntoResponse;
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"hems-days.csv\"",
            ),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        csv(days),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(day: &str, cost: f64, without: f64) -> Receipt {
        Receipt {
            priced: true,
            day: day.into(),
            bill_eur: cost - 0.20,
            wear_eur: 0.15,
            comfort_eur: 0.05,
            cost_eur: cost,
            without_eur: without,
            saved_eur: without - cost,
            on_bill_eur: without - cost + 0.20,
            imported_kwh: 12.3456,
            produced_kwh: 3.2,
            reduced_minutes: Some(90),
            reductions: 1,
            reduction_respected: true,
            feed_in_respected: true,
        }
    }

    #[test]
    fn a_month_is_the_sum_of_its_receipts_to_the_cent() {
        // Thirty days whose euros do not sit on a cent: 1/3 € apart, so thirty
        // rounded-then-summed and summed-then-rounded figures can disagree.
        let days: Vec<ClosedDay> = (1..=30)
            .map(|d| {
                let cost = 3.0 + f64::from(d) / 3.0;
                let without = 4.0 + f64::from(d) * 0.3777;
                ClosedDay::of(&receipt(&format!("2026-11-{d:02}"), cost, without), true)
            })
            .collect();
        let by_hand: i64 = days.iter().map(|d| d.money.expect("priced").saved_ct).sum();
        let month = Days::of(days.clone());
        assert_eq!(month.total.days, 30);
        assert_eq!(month.total.priced_days, 30);
        assert_eq!(month.total.money.saved_ct, by_hand);
        // And each day's own subtraction holds in cents, so the total's does.
        for d in &days {
            let m = d.money.expect("priced");
            assert_eq!(m.saved_ct, m.without_ct - m.cost_ct);
        }
        assert_eq!(
            month.total.money.saved_ct,
            month.total.money.without_ct - month.total.money.cost_ct
        );
    }

    #[test]
    fn a_metered_day_has_no_euros_and_adds_none() {
        // A box on a wall has no counterfactual: its receipt's euros are zero
        // because nothing was modelled, and the month must not count them.
        let metered = ClosedDay::of(&receipt("2026-11-02", 0.0, 0.0), false);
        assert_eq!(metered.money, None);
        let priced = ClosedDay::of(&receipt("2026-11-01", 4.0, 5.07), true);
        let month = Days::of(vec![priced, metered]);
        assert_eq!(month.total.days, 2);
        assert_eq!(month.total.priced_days, 1);
        assert_eq!(month.total.money.saved_ct, 107);
        assert_eq!(month.total.imported_wh, 2 * 12_346);
    }

    #[test]
    fn a_window_keeps_its_gaps() {
        let days = vec![
            ClosedDay::of(&receipt("2026-11-01", 4.0, 5.0), true),
            // The 2nd is missing: the box was off.
            ClosedDay::of(&receipt("2026-11-03", 4.0, 5.0), true),
            ClosedDay::of(&receipt("2026-11-04", 4.0, 5.0), true),
        ];
        let window = Days::within(days, parse_day("2026-11-02"), parse_day("2026-11-03"));
        assert_eq!(window.days.len(), 1);
        assert_eq!(window.days[0].day, "2026-11-03");
    }

    #[test]
    fn a_window_that_is_not_dates_or_runs_backwards_is_refused() {
        let ask = |from: &str, to: &str| Window {
            from: Some(from.into()),
            to: Some(to.into()),
        };
        assert!(ask("2026-11-01", "2026-11-30").of(Vec::new()).is_some());
        assert!(ask("2026-11-30", "2026-11-01").of(Vec::new()).is_none());
        assert!(ask("yesterday", "2026-11-01").of(Vec::new()).is_none());
        assert!(Window::default().of(Vec::new()).is_some());
    }

    #[test]
    fn the_csv_is_one_row_per_day_with_a_decimal_point() {
        let days = vec![
            ClosedDay::of(&receipt("2026-11-01", 4.0, 5.07), true),
            ClosedDay::of(&receipt("2026-11-02", 0.0, 0.0), false),
        ];
        let text = csv(&days);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], CSV_HEADER);
        assert_eq!(
            lines[1],
            "2026-11-01,3.80,0.15,0.05,4.00,5.07,1.07,1.27,12.346,3.200,90,1,true,true"
        );
        assert_eq!(lines[2], "2026-11-02,,,,,,,,12.346,3.200,90,1,true,true");
        let columns = CSV_HEADER.split(',').count();
        assert!(lines.iter().all(|l| l.split(',').count() == columns));
    }

    #[test]
    fn a_negative_saving_keeps_its_sign_in_the_csv() {
        assert_eq!(decimal(-5, 2), "-0.05");
        assert_eq!(decimal(-175, 2), "-1.75");
        assert_eq!(decimal(0, 3), "0.000");
    }

    #[test]
    fn a_day_that_is_not_a_date_is_never_written_as_text() {
        let mut odd = ClosedDay::of(&receipt("2026-11-01", 4.0, 5.0), true);
        odd.day = "=HYPERLINK(\"http://x\")".into();
        assert_eq!(csv(&[odd]).lines().count(), 1);
    }
}
