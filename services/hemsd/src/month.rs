//! The closed days the box keeps, and what they add up to.
//!
//! Money is rounded once at day close. Days without measurements are absent.

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

/// Share of observed quarter hours inside the published forecast band, in percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ForecastCoverage {
    /// Solar production observations inside the forecast band.
    pub pv_pct: u8,
    /// Household load observations inside the forecast band.
    pub load_pct: u8,
}

impl ForecastCoverage {
    /// Round the two observed coverage fractions for presentation.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(crate) fn of(pv: f64, load: f64) -> Option<Self> {
        let percent = |fraction: f64| {
            (0.0..=1.0)
                .contains(&fraction)
                .then(|| (100.0 * fraction).round() as u8)
        };
        Some(Self {
            pv_pct: percent(pv)?,
            load_pct: percent(load)?,
        })
    }
}

/// Prices in force for a fully observed connection-meter interval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntervalTerms {
    /// Signed supply contract used for this interval.
    pub contract: Option<String>,
    /// Gross import price, cents per kWh.
    #[serde(with = "rust_decimal::serde::str")]
    pub import_ct: rust_decimal::Decimal,
    /// Actual export remuneration for this interval in cents per kWh, where known.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub export_ct: Option<rust_decimal::Decimal>,
    /// Annual standing charges and the tariff's fixed network adjustment, euros.
    #[serde(with = "rust_decimal::serde::str")]
    pub annual_fixed_eur: rust_decimal::Decimal,
}

impl IntervalTerms {
    pub(crate) fn net_charge(
        &self,
        draw: rust_decimal::Decimal,
        feed: rust_decimal::Decimal,
    ) -> Option<rust_decimal::Decimal> {
        let import = draw.checked_mul(self.import_ct)?;
        if feed.is_zero() {
            Some(import)
        } else {
            import.checked_sub(feed.checked_mul(self.export_ct?)?)
        }
    }
}

/// Metered charges for one Berlin calendar day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MeteredBill {
    /// Seller contract references in first-use order.
    #[serde(default)]
    pub contracts: Vec<String>,
    /// Calendar intervals, including the shorter or longer clock-change day.
    pub expected_intervals: u16,
    /// Recorded intervals for which another controller retained responsibility.
    pub watched_intervals: u16,
    /// Intervals with a full grid measurement and a known tariff.
    pub priced_intervals: u16,
    /// Variable import charges less export revenue, rounded at day close.
    pub energy_ct: Option<i64>,
    /// Daily share of annual fixed charges, when the whole day was priced.
    pub fixed_ct: Option<i64>,
}

impl MeteredBill {
    pub(crate) fn from_registers(date: Date, rows: &[crate::store::Recorded]) -> Self {
        use rust_decimal::{Decimal, prelude::ToPrimitive};
        let start = ems_core::calendar::day_start_utc(date);
        let slots = (ems_core::calendar::day_end_utc(date) - start).whole_minutes() / 15;
        let mut bill = Self {
            contracts: Vec::new(),
            expected_intervals: u16::try_from(slots).expect("one calendar day"),
            priced_intervals: 0,
            watched_intervals: 0,
            energy_ct: None,
            fixed_ct: None,
        };
        let mut variable = Some(Decimal::ZERO);
        let mut fixed = Some(Decimal::ZERO);
        let year_slots =
            Decimal::from(time::util::days_in_year(date.year())) * Decimal::from(slots);
        let day_offset = i64::from(date.ordinal() - 1) * slots;
        let day = ems_core::prelude::Horizon::new(start, usize::from(bill.expected_intervals));
        let mut intervals = std::collections::BTreeMap::new();
        for row in rows {
            if let Some(i) = day.index_of(row.registers.slot) {
                // Two claims for one interval cannot establish its charge.
                intervals
                    .entry(i)
                    .and_modify(|value| *value = None)
                    .or_insert(Some(row));
            }
        }
        for row in intervals.into_values().flatten() {
            bill.watched_intervals += u16::from(row.watched);
            let Some(terms) = &row.terms else { continue };
            if let Some(contract) = &terms.contract
                && !bill.contracts.contains(contract)
            {
                bill.contracts.push(contract.clone());
            }
            let q = &row.registers;
            let Some(charge) = terms.net_charge(q.grid_draw, q.grid_feed_in) else {
                continue;
            };
            bill.priced_intervals += 1;
            variable = variable.and_then(|sum| sum.checked_add(charge));
            // Differences of cumulative cents allocate every cent once, also
            // when two contracts cover different quarters of this day.
            let n = (q.slot.start() - start).whole_minutes() / 15 + day_offset;
            let accrued = |index| {
                terms
                    .annual_fixed_eur
                    .checked_mul(Decimal::ONE_HUNDRED)?
                    .checked_mul(Decimal::from(index))?
                    .checked_div(year_slots)
                    .map(|v| v.round())
            };
            fixed =
                fixed.and_then(|sum| sum.checked_add(accrued(n + 1)?.checked_sub(accrued(n)?)?));
        }
        if bill.expected_intervals == bill.priced_intervals {
            bill.fixed_ct = fixed.and_then(|v| v.to_i64());
            bill.energy_ct = variable.and_then(|v| v.round().to_i64());
        }
        bill
    }
}

/// One closed day, as the box keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClosedDay {
    /// The local day, `YYYY-MM-DD`.
    pub day: String,
    /// Measured forecast coverage, where the closed day had enough scored observations.
    pub forecast: Option<ForecastCoverage>,
    /// Charges computed from the connection-meter record.
    pub metered_bill: Option<MeteredBill>,
    /// Equal-service comparison where both matched energy traces cover the day.
    pub comparison: Option<crate::runtime::day::replay::Comparison>,
    /// First interval with a recorded reason for withholding the comparison.
    pub comparison_gap: Option<crate::runtime::day::replay::ComparisonGap>,
    /// Grid-charge margin on measured closed battery blocks.
    pub arbitrage: Option<crate::runtime::day::storage::Arbitrage>,
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

impl From<&Receipt> for Money {
    fn from(receipt: &Receipt) -> Self {
        let cost_ct = cents(receipt.cost_eur);
        let without_ct = cents(receipt.without_eur);
        Self {
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
    }
}

impl ClosedDay {
    /// The day a receipt describes. `priced` says whether its euros are a
    /// computed counterfactual or merely absent — a metered day's receipt
    /// carries zeroes where nothing was modelled, and those must not be kept
    /// as a day that cost nothing.
    #[must_use]
    pub fn of(receipt: &Receipt, priced: bool) -> Self {
        let money = priced.then(|| Money::from(receipt));
        Self {
            day: receipt.day.clone(),
            money,
            forecast: None,
            metered_bill: None,
            comparison: None,
            comparison_gap: None,
            arbitrage: None,
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
        let mut receipt = Self::of(&crate::home::receipt_of(day), day.economics.is_some());
        receipt.forecast = day
            .forecast
            .and_then(|s| ForecastCoverage::of(s.pv_coverage, s.load_coverage));
        receipt
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
    /// Number of complete bills contributing to the measured cost.
    pub metered_days: u32,
    /// Variable and fixed charges of those complete bills, in cents.
    pub metered_cost_ct: i128,
    /// Days with a complete matched comparison, including zero and inconclusive differences.
    pub eligible_days: u32,
    /// Days whose matched comparison exceeds its observed model error.
    pub conclusive_days: u32,
    /// Signed difference on those days. A loss reduces the sum.
    pub compared_delta_ct: i128,
    /// Total observed replay error of the same conclusive days.
    pub compared_error_ct: i128,
    /// Number of measured, closed grid-charge blocks.
    pub grid_charge_blocks: u32,
    /// Avoided import costs less grid-charge costs on those blocks, in cents.
    pub grid_charge_margin_ct: i128,
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
            if let Some(bill) = &day.metered_bill
                && let (Some(variable), Some(fixed)) = (bill.energy_ct, bill.fixed_ct)
            {
                total.metered_days += 1;
                total.metered_cost_ct += i128::from(variable) + i128::from(fixed);
            }
            if let Some(a) = day.arbitrage {
                total.grid_charge_blocks += u32::from(a.blocks);
                total.grid_charge_margin_ct += i128::from(a.avoided_ct) - i128::from(a.bought_ct);
            }
            total.eligible_days += u32::from(day.comparison.is_some());
            if let Some(c) = day
                .comparison
                .filter(crate::runtime::day::replay::Comparison::conclusive)
            {
                total.conclusive_days += 1;
                total.compared_delta_ct += c.delta();
                total.compared_error_ct += i128::from(c.error_ct);
            }
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
feed_in_respected,metered_energy_eur,fixed_charges_eur,priced_intervals,expected_intervals,watched_intervals,with_plan_eur,without_plan_eur,replay_error_eur,conclusive,grid_charge_blocks,bought_kwh,served_kwh,grid_charge_eur,avoided_import_eur,grid_charge_margin_eur,buy_ct_per_kwh,use_ct_per_kwh,comparison_gap_at_unix_s,comparison_gap_reason";

/// Whole hundredths or thousandths as a decimal, exactly.
pub(crate) fn decimal(value: impl Into<i128>, places: u32) -> String {
    let value = value.into();
    let scale = 10_i128.pow(places);
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
            d.metered_bill
                .as_ref()
                .and_then(|b| b.energy_ct)
                .map_or_else(String::new, |v| decimal(v, 2)),
            d.metered_bill
                .as_ref()
                .and_then(|b| b.fixed_ct)
                .map_or_else(String::new, |v| decimal(v, 2)),
            d.metered_bill
                .as_ref()
                .map_or_else(String::new, |b| b.priced_intervals.to_string()),
            d.metered_bill
                .as_ref()
                .map_or_else(String::new, |b| b.expected_intervals.to_string()),
            d.metered_bill
                .as_ref()
                .map_or_else(String::new, |b| b.watched_intervals.to_string()),
            d.comparison
                .map_or_else(String::new, |c| decimal(c.with_plan_ct, 2)),
            d.comparison
                .map_or_else(String::new, |c| decimal(c.without_plan_ct, 2)),
            d.comparison
                .map_or_else(String::new, |c| decimal(c.error_ct, 2)),
            d.comparison
                .map_or_else(String::new, |c| c.conclusive().to_string()),
            d.arbitrage
                .map_or_else(String::new, |a| a.blocks.to_string()),
            d.arbitrage
                .map_or_else(String::new, |a| decimal(a.bought_wh, 3)),
            d.arbitrage
                .map_or_else(String::new, |a| decimal(a.served_wh, 3)),
            d.arbitrage
                .map_or_else(String::new, |a| decimal(a.bought_ct, 2)),
            d.arbitrage
                .map_or_else(String::new, |a| decimal(a.avoided_ct, 2)),
            d.arbitrage.map_or_else(String::new, |a| {
                decimal(i128::from(a.avoided_ct) - i128::from(a.bought_ct), 2)
            }),
            d.arbitrage
                .map_or_else(String::new, |a| decimal(a.buy_rate_mct_per_kwh, 3)),
            d.arbitrage
                .map_or_else(String::new, |a| decimal(a.use_rate_mct_per_kwh, 3)),
            d.comparison_gap
                .map_or_else(String::new, |g| g.at_unix_s.to_string()),
            // The JSON spelling of this closed enum is also a quoted CSV field.
            d.comparison_gap.map_or_else(String::new, |g| {
                serde_json::to_string(&g.reason).unwrap_or_default()
            }),
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
            "2026-11-01,3.80,0.15,0.05,4.00,5.07,1.07,1.27,12.346,3.200,90,1,true,true,,,,,,,,,,,,,,,,,,,"
        );
        assert_eq!(
            lines[2],
            "2026-11-02,,,,,,,,12.346,3.200,90,1,true,true,,,,,,,,,,,,,,,,,,,"
        );
        let columns = CSV_HEADER.split(',').count();
        assert!(lines.iter().all(|l| l.split(',').count() == columns));
        let mut excluded = days[1].clone();
        excluded.comparison_gap = Some(crate::runtime::day::replay::ComparisonGap {
            at_unix_s: 1_793_581_200,
            reason: crate::runtime::day::replay::GapReason::VehicleEnergy,
        });
        assert!(csv(&[excluded]).ends_with(",1793581200,\"vehicle_energy\"\n"));
    }

    #[test]
    fn a_bill_requires_a_whole_local_day_at_known_prices() {
        use rust_decimal::Decimal;
        use time::macros::date;
        for (date, count) in [
            (date!(2026 - 03 - 29), 92_u16),
            (date!(2026 - 10 - 25), 100),
        ] {
            let first =
                ems_core::prelude::Slot::containing(ems_core::calendar::day_start_utc(date));
            let mut rows: Vec<_> = (0..count)
                .map(|i| {
                    let mut registers =
                        ems_grid::mispel::QuarterHour::empty(first.offset(i64::from(i)));
                    registers.grid_draw = Decimal::new(1, 1);
                    registers.grid_feed_in = Decimal::new(1, 2);
                    registers.anzulegender_wert = Some(Decimal::new(8, 0));
                    crate::store::Recorded {
                        registers,
                        production: None,
                        watched: false,
                        replay: None,
                        storage: None,
                        terms: Some(IntervalTerms {
                            export_ct: Some(Decimal::new(8, 0)),
                            contract: None,
                            import_ct: Decimal::new(30, 0),
                            annual_fixed_eur: Decimal::new(365, 0),
                        }),
                    }
                })
                .collect();
            let bill = MeteredBill::from_registers(date, &rows);
            assert_eq!(bill.expected_intervals, count);
            assert_eq!(bill.priced_intervals, count);
            assert_eq!(bill.energy_ct, Some(if count == 92 { 269 } else { 292 }));
            assert_eq!(bill.fixed_ct, Some(100));
            for (i, row) in rows.iter_mut().enumerate() {
                let terms = row.terms.as_mut().unwrap();
                if i < usize::from(count / 2) {
                    terms.contract = Some("First".into());
                } else {
                    terms.contract = Some("Replacement".into());
                    terms.annual_fixed_eur *= Decimal::TWO;
                }
            }
            let switched = MeteredBill::from_registers(date, &rows);
            assert_eq!(switched.contracts, ["First", "Replacement"]);
            assert_eq!(switched.energy_ct, bill.energy_ct);
            assert_eq!(switched.fixed_ct, Some(150));
            for wrong_slot in [
                first.offset(-1),
                first.offset(1),
                first.offset(i64::from(count)),
            ] {
                rows[0].registers.slot = wrong_slot;
                let incomplete = MeteredBill::from_registers(date, &rows);
                assert!(incomplete.priced_intervals < count);
                assert!(incomplete.energy_ct.is_none() && incomplete.fixed_ct.is_none());
            }
            rows[0].registers.slot = first;
            rows[0].terms.as_mut().unwrap().export_ct = None;
            let unpriced = MeteredBill::from_registers(date, &rows);
            assert_eq!(unpriced.priced_intervals, count - 1);
            assert!(unpriced.energy_ct.is_none());
            rows[0].terms = None;
            let gap = MeteredBill::from_registers(date, &rows);
            assert_eq!(gap.priced_intervals, count - 1);
            assert!(gap.energy_ct.is_none() && gap.fixed_ct.is_none());
        }
    }

    #[test]
    fn a_forecast_day_keeps_only_defined_fractions() {
        assert_eq!(
            ForecastCoverage::of(0.0, 0.816),
            Some(ForecastCoverage {
                pv_pct: 0,
                load_pct: 82
            })
        );
        assert_eq!(ForecastCoverage::of(f64::NAN, 0.8), None);
        assert_eq!(ForecastCoverage::of(0.8, 1.1), None);
    }

    #[test]
    fn only_differences_above_error_count_and_battery_margin_stays_separate() {
        use crate::runtime::day::{replay::Comparison, storage::Arbitrage};
        let mut receipts = Vec::new();
        for (day, with, error) in [("01", 80, 2), ("02", 110, 1), ("03", 90, 10)] {
            let mut r = ClosedDay::of(&receipt(&format!("2026-11-{day}"), 0., 0.), false);
            r.comparison = Some(Comparison {
                with_plan_ct: with,
                without_plan_ct: 100,
                error_ct: error,
            });
            receipts.push(r);
        }
        receipts[0].arbitrage = Some(Arbitrage {
            blocks: 1,
            bought_wh: 1000,
            served_wh: 800,
            bought_ct: 30,
            buy_rate_mct_per_kwh: 30_000,
            use_rate_mct_per_kwh: 35_000,
            avoided_ct: 28,
        });
        let month = Days::of(receipts);
        assert_eq!(
            (
                month.total.conclusive_days,
                month.total.compared_delta_ct,
                month.total.compared_error_ct
            ),
            (2, 10, 3)
        );
        assert_eq!(month.total.eligible_days, 3);
        assert_eq!(month.total.grid_charge_margin_ct, -2);
        assert!(csv(&month.days).contains("0.90,1.00,0.10,false,"));
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
