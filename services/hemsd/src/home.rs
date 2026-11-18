//! The household page's one view of the box.
//!
//! The box already knows what a household wants to know — what each device is
//! doing, why, what the network operator has asked of the house, what tonight
//! will cost — and every one of those facts is scattered over `/v1/status`, the
//! plan and the day report. [`Home`] puts them in the shape a person reads them
//! in, and it is built by pure functions so that the running box and
//! `hemsd demo` produce the *same* view from the same kind of input.
//!
//! # Explanations are keys, not prose
//!
//! Every setpoint carries a [`Reason`] (`Setpoint::new(value, reason)` is the only
//! constructor), so "why" is never invented: it is that reason, rendered. The box
//! sends a key and its parameters — `plan.charge` with `price_ct = 15` — and the
//! page owns the sentence in German and English. Two consequences, both
//! deliberate: the wording can be improved without touching the control stack,
//! and a reason the page has no sentence for fails a test rather than showing a
//! household an identifier ([`ALL_KEYS`] is held against the page's templates).
//!
//! # Nothing here decides anything
//!
//! A view. The only judgement in the module is [`judge_goal`], and it asks the
//! planner's own session model rather than a second heuristic.

use std::collections::BTreeMap;

use hems_core::prelude::{Asset, AssetId, Power, Site, Slot};
use hems_core::setpoint::{FallbackCause, GuardRule, RealtimeCause, Reason, UserOverride};
use serde::Serialize;

/// Who is responsible for what a device is doing — the colour the page gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// The network operator: a § 14a reduction, a § 9 EEG feed-in limit, the
    /// failsafe that stands in for one.
    Operator,
    /// A physical or legal limit of the installation itself.
    Safety,
    /// The household asked for it.
    Household,
    /// The box's own plan.
    Plan,
    /// An energy manager the household connected.
    Manager,
    /// The second-by-second controller, following what the house is doing now.
    Live,
    /// The box has no plan it trusts and is being conservative.
    Fallback,
}

/// Why a device is doing what it is doing, as a template key and its parameters.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Explanation {
    /// Who is responsible.
    pub cause: Cause,
    /// The sentence to render, e.g. `plan.charge`. One of [`ALL_KEYS`].
    pub key: &'static str,
    /// The values the sentence names, already formatted.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<&'static str, String>,
    /// The regulation, where a rule is the reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation: Option<&'static str>,
}

/// Every key [`explain`] can produce. The page must have a sentence for each, in
/// each language it speaks; a test reads the page and checks.
pub const ALL_KEYS: &[&str] = &[
    "guard.lpc",
    "guard.lpp",
    "guard.para9_cap",
    "guard.failsafe",
    "guard.circuit_limit",
    "guard.contract_limit",
    "guard.unbalance",
    "guard.device_limit",
    "guard.backup_reserve",
    "user.boost",
    "user.pause",
    "user.away",
    "plan.charge",
    "plan.run",
    "plan.discharge",
    "plan.wait",
    "manager",
    "live.surplus_tracking",
    "live.self_consumption",
    "live.phase_balance",
    "live.ramp_limit",
    "live.hysteresis",
    "live.local_control",
    "live.maximum_power_point",
    "fallback.no_plan",
    "fallback.plan_stale",
    "fallback.driver_lost",
    "fallback.clock_unsynchronised",
    "idle",
];

/// Render a setpoint's reason as an explanation.
///
/// `power` is what the device was told (load convention: positive draws), and
/// `price_ct` the all-in import price of the current slot where one is known.
/// Exhaustive on purpose: a new [`Reason`], [`GuardRule`] or cause does not
/// compile until it has a key here, and the key does not pass until the page has
/// its sentence.
#[must_use]
pub fn explain(reason: &Reason, power: Power, price_ct: Option<f64>) -> Explanation {
    let mut params = BTreeMap::new();
    if let Some(p) = price_ct {
        params.insert("price_ct", format!("{p:.0}"));
    }
    let (cause, key, citation) = match reason {
        Reason::Guard { rule, .. } => match rule {
            GuardRule::Lpc => (
                Cause::Operator,
                "guard.lpc",
                Some("§ 14a EnWG · BK6-22-300 Anlage 1, Ziff. 4"),
            ),
            GuardRule::Lpp => (Cause::Operator, "guard.lpp", Some("§ 9 EEG")),
            GuardRule::Para9Cap => (Cause::Operator, "guard.para9_cap", Some("§ 9 Abs. 2 EEG")),
            GuardRule::Failsafe => (
                Cause::Operator,
                "guard.failsafe",
                Some("§ 14a EnWG · EEBUS LPC failsafe"),
            ),
            GuardRule::CircuitLimit => (Cause::Safety, "guard.circuit_limit", None),
            GuardRule::ContractLimit => (Cause::Safety, "guard.contract_limit", None),
            GuardRule::Unbalance => (Cause::Safety, "guard.unbalance", Some("VDE-AR-N 4100")),
            GuardRule::DeviceLimit => (Cause::Safety, "guard.device_limit", None),
            GuardRule::BackupReserve => (Cause::Household, "guard.backup_reserve", None),
        },
        Reason::User(what) => (
            Cause::Household,
            match what {
                UserOverride::Boost => "user.boost",
                UserOverride::Pause => "user.pause",
                UserOverride::Away => "user.away",
            },
            None,
        ),
        Reason::Plan { .. } => {
            let kw = power.kw();
            let key = if kw > 0.05 {
                "plan.charge"
            } else if kw < -0.05 {
                "plan.discharge"
            } else {
                "plan.wait"
            };
            (Cause::Plan, key, None)
        }
        Reason::Cem => (Cause::Manager, "manager", None),
        Reason::Realtime(cause) => (
            Cause::Live,
            match cause {
                RealtimeCause::SurplusTracking => "live.surplus_tracking",
                RealtimeCause::SelfConsumption => "live.self_consumption",
                RealtimeCause::PhaseBalance => "live.phase_balance",
                RealtimeCause::RampLimit => "live.ramp_limit",
                RealtimeCause::Hysteresis => "live.hysteresis",
                RealtimeCause::LocalControl => "live.local_control",
                RealtimeCause::MaximumPowerPoint => "live.maximum_power_point",
            },
            None,
        ),
        Reason::Fallback(cause) => (
            Cause::Fallback,
            match cause {
                FallbackCause::NoPlan => "fallback.no_plan",
                FallbackCause::PlanStale => "fallback.plan_stale",
                FallbackCause::DriverLost => "fallback.driver_lost",
                FallbackCause::ClockUnsynchronised => "fallback.clock_unsynchronised",
            },
            None,
        ),
    };
    if power.kw().abs() > 0.05 {
        params.insert("kw", format!("{:.1}", power.kw().abs()));
    }
    Explanation {
        cause,
        key,
        params,
        citation,
    }
}

/// What the page says about a device that was told nothing this tick.
#[must_use]
pub fn idle() -> Explanation {
    Explanation {
        cause: Cause::Plan,
        key: "idle",
        params: BTreeMap::new(),
        citation: None,
    }
}

/// One device, as the page shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Device {
    /// The asset.
    pub id: String,
    /// The household's own name for it, where the configuration gives one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `car`, `heat_pump`, `battery`, `hot_water`, `roof`, `appliance` — the
    /// page names and draws it from this.
    pub kind: &'static str,
    /// What it draws (positive) or delivers (negative), kW, where measured or
    /// commanded.
    pub kw: Option<f64>,
    /// State of charge in `[0, 1]`, where the device has one and reports it.
    pub soc: Option<f64>,
    /// Why it is doing it.
    pub why: Explanation,
    /// Something the household should know about the device itself: `silent`
    /// (nothing heard), `assumed` (a nameplate stands in for a reading),
    /// `disobedient` (acknowledged a setpoint and did not act), `undriven` (no
    /// driver speaks for it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<&'static str>,
}

/// The kind of device an asset is, for the page.
#[must_use]
pub fn kind_of(asset: &Asset) -> Option<&'static str> {
    Some(match asset {
        Asset::Pv(_) => "roof",
        Asset::Battery(_) => "battery",
        Asset::Evse(_) => "car",
        Asset::HeatPump(_) => "heat_pump",
        Asset::Dhw(_) => "hot_water",
        // The house's own uninstrumented load is the house, not a device a
        // household can do anything about.
        Asset::Load(load) if matches!(load.kind, hems_core::asset::LoadKind::Fixed) => return None,
        Asset::Load(_) => "appliance",
        Asset::Meter(_) | Asset::Relay(_) => return None,
    })
}

/// What the network operator has asked of the house right now — `None` when it
/// has asked nothing.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reduction {
    /// The ceiling on the controllable devices, kW.
    pub ceiling_kw: f64,
    /// What the household is guaranteed whatever the ceiling, kW — `[A1 4.5]`.
    pub minimum_kw: Option<f64>,
    /// What the controllable devices are drawing against it, kW.
    pub drawing_kw: Option<f64>,
    /// `true` while the box is in failsafe: no heartbeat from the Steuerbox, so
    /// the stored failsafe value stands in for an instruction.
    pub failsafe: bool,
    /// How each device's share of the allowance is being spent, kW.
    pub shares: BTreeMap<String, f64>,
}

/// One quarter hour of the plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanSlot {
    /// Its start, RFC 3339.
    pub start: String,
    /// The all-in import price, ct/kWh, where known.
    pub price_ct: Option<f64>,
    /// What the plan has each device do, kW.
    pub kw: BTreeMap<String, f64>,
    /// `true` while a § 14a reduction is expected or in force in this slot.
    pub reduced: bool,
}

/// The day so far, as a receipt.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Receipt {
    /// The local day, `YYYY-MM-DD`.
    pub day: String,
    /// What the household paid for electricity, €.
    pub bill_eur: f64,
    /// Battery wear priced, €.
    pub wear_eur: f64,
    /// Comfort given up, priced, €.
    pub comfort_eur: f64,
    /// Bill + wear + comfort, €.
    pub cost_eur: f64,
    /// The same house with the same hardware and no energy manager, €.
    pub without_eur: f64,
    /// `without − cost`, €.
    pub saved_eur: f64,
    /// The part of the saving that reached the bill, €.
    pub on_bill_eur: f64,
    /// Bought from the grid, kWh.
    pub imported_kwh: f64,
    /// Made by the roof, kWh.
    pub produced_kwh: f64,
    /// Minutes under a § 14a reduction, where the day measured them.
    pub reduced_minutes: Option<i64>,
    /// How many reductions the network operator sent.
    pub reductions: usize,
    /// Whether every reduction was respected.
    pub reduction_respected: bool,
    /// Whether the feed-in ceiling was respected.
    pub feed_in_respected: bool,
}

/// What a car goal comes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Already there.
    Met,
    /// Comfortably possible.
    Feasible,
    /// Possible only if nothing else needs the connection.
    Tight,
    /// Not possible by then.
    Impossible,
}

/// The answer a household gets the moment it sets a goal.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GoalAnswer {
    /// The verdict.
    pub verdict: Verdict,
    /// What still has to go in, kWh.
    pub needed_kwh: f64,
    /// The share of the time left the goal would use: `0` nothing, `1` all of it.
    pub tightness: f64,
    /// Where the goal is impossible: the state of charge the car can reach by
    /// then, in `[0, 1]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reachable_soc: Option<f64>,
}

/// Above this share of the time left, a goal is called tight.
///
/// Chosen so a goal needing more than four fifths of every remaining slot at full
/// power is flagged: it survives only if no § 14a reduction, phase switch or
/// other device takes a slot from it.
pub const TIGHT_ABOVE: f64 = 0.8;

/// Judge a car goal with the planner's own session model.
///
/// `session` is the session the planner would plan: what is in the car, the goal
/// as target and departure, the charge point's limits.
#[must_use]
pub fn judge_goal(session: &hems_optimizer::model::EvSession, now: Slot) -> GoalAnswer {
    let needed = (session.energy_target - session.energy_now).kwh().max(0.0);
    if needed <= 0.0 {
        return GoalAnswer {
            verdict: Verdict::Met,
            needed_kwh: 0.0,
            tightness: 0.0,
            reachable_soc: None,
        };
    }
    let tightness = demand_ratio(session, now);
    let verdict = if tightness > 1.0 {
        Verdict::Impossible
    } else if tightness > TIGHT_ABOVE {
        Verdict::Tight
    } else {
        Verdict::Feasible
    };
    let reachable_soc =
        (verdict == Verdict::Impossible && session.capacity.kwh() > 0.0).then(|| {
            let deliverable = needed / tightness;
            ((session.energy_now.kwh() + deliverable) / session.capacity.kwh()).clamp(0.0, 1.0)
        });
    GoalAnswer {
        verdict,
        needed_kwh: needed,
        tightness,
        reachable_soc,
    }
}

/// What a goal owes against what the time left can deliver at full power.
///
/// `EvSession::tightness` is this ratio clamped to `[0, 1]` — right for the
/// planner, which only asks how much of the window a promise uses, and blind
/// above one, which is exactly where a goal becomes impossible. The same
/// arithmetic, unclamped; the test holds the two together below one.
fn demand_ratio(session: &hems_optimizer::model::EvSession, now: Slot) -> f64 {
    let owed = (session.energy_target - session.energy_now).kwh().max(0.0);
    let from = session.arrival.unwrap_or(now).max(now);
    let slots = from.distance_to(session.departure).max(0) as f64;
    let deliverable = session.max_charge.kw() * session.efficiency.clamp(0.0, 1.0) * slots * 0.25;
    if deliverable <= 0.0 {
        return f64::INFINITY;
    }
    owed / deliverable
}

/// A car goal as the household set it.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CarGoal {
    /// The state of charge wanted, in `[0, 1]`.
    pub target_soc: f64,
    /// By when, local wall-clock minutes after midnight (`420` is 07:00).
    pub by_minute: u16,
}

/// The household's car goal, shared between the page and the planner.
pub type Goals = std::sync::Arc<tokio::sync::Mutex<Option<CarGoal>>>;

/// The car as the planner last saw it — what is in it, its capacity, the charge
/// point's limits — published so a new goal can be judged at once, without a
/// solve. Its target and departure are placeholders; [`session_for`] sets them.
pub type CarFacts = std::sync::Arc<tokio::sync::Mutex<Option<hems_optimizer::model::EvSession>>>;

/// The next instant the Berlin wall clock reads `by_minute` after `now`.
///
/// Wall-clock, not elapsed time: on the day the clocks go forward 07:00 is six
/// hours after midnight, not seven.
#[must_use]
pub fn next_wall_clock(by_minute: u16, now: time::OffsetDateTime) -> time::OffsetDateTime {
    use hems_core::calendar::{day_start_utc, local_day, to_berlin};
    let at = |day: time::Date| {
        let start = day_start_utc(day);
        let naive = start + time::Duration::minutes(i64::from(by_minute));
        let shift =
            to_berlin(naive).offset().whole_seconds() - to_berlin(start).offset().whole_seconds();
        naive - time::Duration::seconds(i64::from(shift))
    };
    let today = local_day(now);
    let candidate = at(today);
    if candidate > now {
        candidate
    } else {
        at(today.next_day().unwrap_or(today))
    }
}

/// The session the planner would plan for `goal`, from the car as last seen.
#[must_use]
pub fn session_for(
    facts: &hems_optimizer::model::EvSession,
    goal: CarGoal,
    now: time::OffsetDateTime,
) -> hems_optimizer::model::EvSession {
    hems_optimizer::model::EvSession {
        energy_target: facts.capacity * goal.target_soc.clamp(0.0, 1.0),
        departure: Slot::containing(next_wall_clock(goal.by_minute, now)),
        arrival: None,
        ..*facts
    }
}

/// Everything the household page shows, in one document.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Home {
    /// Which household.
    pub site: String,
    /// The instant this view describes, RFC 3339.
    pub at: Option<String>,
    /// `true` when this is `hemsd demo` replaying a reference day.
    pub demo: bool,
    /// In a demo, the minute of the day being shown, so the page's slider can
    /// start where the replay is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_minute: Option<u16>,
    /// From the grid right now, kW (negative: feeding in).
    pub grid_kw: Option<f64>,
    /// The all-in import price right now, ct/kWh.
    pub price_ct: Option<f64>,
    /// The network operator's reduction, while one is in force.
    pub reduction: Option<Reduction>,
    /// Every device the household would recognise.
    pub devices: Vec<Device>,
    /// The plan, from now to the end of its horizon.
    pub plan: Vec<PlanSlot>,
    /// What the plan expects its horizon to cost, and the same horizon without an
    /// energy manager, €.
    pub plan_cost_eur: Option<(f64, f64)>,
    /// The day so far.
    pub receipt: Option<Receipt>,
    /// The car goal in force, and what it comes to.
    pub goal: Option<(CarGoal, GoalAnswer)>,
    /// What an installer gives the metering point operator.
    pub ski: Option<String>,
    /// The SHA-256 fingerprint of the box's SHIP certificate.
    pub fingerprint: Option<String>,
    /// The SHA-256 fingerprint of the certificate it serves HTTPS with — what
    /// a browser or an S2 manager pins.
    pub https_fingerprint: Option<String>,
    /// Whether the roof still delivers what it used to: `healthy`, `degraded`,
    /// `learning`.
    pub roof: Option<String>,
}

/// Everything [`live`] reads, borrowed from the running box.
pub struct Live<'a> {
    /// The house.
    pub site: &'a Site,
    /// What the last tick decided and saw.
    pub status: &'a crate::runtime::control::Status,
    /// The plan in force.
    pub plan: Option<&'a hems_core::prelude::Plan>,
    /// The prices it was made against.
    pub prices: Option<&'a hems_tariff::PriceStack>,
    /// The household's car goal.
    pub goal: Option<CarGoal>,
    /// The car as the planner last saw it.
    pub car: Option<hems_optimizer::model::EvSession>,
    /// The box's SKI and certificate fingerprint.
    pub ski: Option<String>,
    /// The SHA-256 fingerprint of its certificate.
    pub fingerprint: Option<String>,
    /// The SHA-256 fingerprint of the certificate it serves HTTPS with.
    pub https_fingerprint: Option<String>,
}

/// The household page's view of a running box.
#[must_use]
pub fn live(input: &Live<'_>, now: time::OffsetDateTime) -> Home {
    let status = input.status;
    let slot = Slot::containing(now);
    let price_of = |slot: Slot| {
        input
            .prices
            .and_then(|p| p.at(slot))
            // `import_f64` is euros per kilowatt-hour; the page speaks cents.
            .map(|p| p.import_f64() * 100.0)
    };
    let price_ct = price_of(slot);
    // What "cheap" means is this household's own next two days, not a number:
    // an all-in price of 36 ct can be the cheapest hour of a winter week.
    let horizon_prices: Vec<f64> = input
        .plan
        .map(|p| {
            p.slots
                .iter()
                .filter(|s| s.slot >= slot)
                .filter_map(|s| price_of(s.slot))
                .collect()
        })
        .unwrap_or_default();

    let devices = devices_now(input, price_ct, &horizon_prices);

    // The guard's own flag: the control box is silent and the stored
    // failsafe value stands in for an instruction.
    let failsafe = status.in_failsafe;
    let reduction = status.steuve_ceiling.map(|ceiling| Reduction {
        ceiling_kw: ceiling.kw(),
        minimum_kw: status.minimum.map(Power::kw),
        drawing_kw: status.netzwirksam.map(Power::kw),
        failsafe,
        shares: status
            .reasons
            .iter()
            .filter(|(_, r)| {
                matches!(
                    r,
                    Reason::Guard {
                        rule: GuardRule::Lpc | GuardRule::Failsafe,
                        ..
                    }
                )
            })
            .filter_map(|(id, _)| Some((id.to_string(), status.commanded.get(id)?.kw())))
            .collect(),
    });

    let plan = input
        .plan
        .map(|p| {
            p.slots
                .iter()
                .filter(|s| s.slot >= slot)
                .map(|s| PlanSlot {
                    start: rfc3339(s.slot.start()),
                    price_ct: price_of(s.slot),
                    kw: s
                        .targets
                        .iter()
                        .map(|t| (t.asset.to_string(), t.power.kw()))
                        .collect(),
                    // Only the quarter hour in force: the plan carries the current
                    // ceiling across its horizon as an assumption, and the
                    // operator announces no end — drawing it as a schedule
                    // would invent one.
                    reduced: s.slot == slot && status.steuve_ceiling.is_some(),
                })
                .collect()
        })
        .unwrap_or_default();

    let goal = input.goal.map(|g| {
        let answer = input.car.map_or(
            GoalAnswer {
                verdict: Verdict::Feasible,
                needed_kwh: 0.0,
                tightness: 0.0,
                reachable_soc: None,
            },
            |facts| judge_goal(&session_for(&facts, g, now), slot),
        );
        (g, answer)
    });

    Home {
        // The household's own name for its house; empty where it gave none,
        // and the page then says "Home" rather than an identifier.
        site: input.site.label.trim().to_owned(),
        at: status.at.map(rfc3339),
        demo: false,
        at_minute: None,
        grid_kw: status.grid.map(Power::kw),
        price_ct,
        reduction,
        devices,
        plan,
        plan_cost_eur: status.plan_expected_eur.zip(status.plan_baseline_eur),
        receipt: status.last_day.as_ref().map(receipt_of),
        goal,
        ski: input.ski.clone(),
        fingerprint: input.fingerprint.clone(),
        https_fingerprint: input.https_fingerprint.clone(),
        roof: status.roof_health.as_ref().map(roof_word),
    }
}

/// Every device the household would recognise, with what it is doing and why.
fn devices_now(input: &Live<'_>, price_ct: Option<f64>, horizon_prices: &[f64]) -> Vec<Device> {
    let status = input.status;
    devices_of(input.site)
        .into_iter()
        .map(|(id, kind)| {
            let (measured, soc) = status.readings.get(&id).copied().unwrap_or((None, None));
            let commanded = status.commanded.get(&id).copied();
            let shown = measured.or(commanded);
            let why = status.reasons.get(&id).map_or_else(idle, |reason| {
                relative(
                    explain(reason, commanded.unwrap_or(Power::ZERO), price_ct),
                    price_ct,
                    horizon_prices,
                )
            });
            let notice = if status.undriven.contains(&id) {
                Some("undriven")
            } else if status.silent.contains(&id) {
                Some("silent")
            } else if status.disobedient.contains_key(&id) {
                Some("disobedient")
            } else if status.assumed_nominal.contains(&id) || status.assumed_available.contains(&id)
            {
                Some("assumed")
            } else {
                None
            };
            Device {
                id: id.to_string(),
                name: input
                    .site
                    .asset(&id)
                    .map(|a| a.meta().label.trim().to_owned())
                    // A label that only repeats the id names nothing.
                    .filter(|l| !l.is_empty() && *l != id.to_string()),
                kind,
                kw: shown.map(Power::kw),
                soc,
                why,
                notice,
            }
        })
        .collect()
}

/// Say *how* cheap a planned charge is, against the plan's own prices.
///
/// `plan.charge` claims the hour is among the cheapest, so it is only sent
/// where it is: in the cheaper half of the horizon, with the share named.
/// Otherwise the device runs because the plan needs it then, and says so.
fn relative(mut e: Explanation, price_ct: Option<f64>, horizon: &[f64]) -> Explanation {
    if e.key != "plan.charge" {
        return e;
    }
    let Some(price) = price_ct.filter(|_| !horizon.is_empty()) else {
        e.key = "plan.run";
        return e;
    };
    let cheaper_or_equal = horizon.iter().filter(|p| **p <= price + 1e-9).count();
    #[allow(clippy::cast_precision_loss)]
    let share = (cheaper_or_equal as f64 / horizon.len() as f64 * 100.0).ceil();
    if share <= 50.0 {
        e.params.insert("among", format!("{share:.0}"));
    } else {
        e.key = "plan.run";
    }
    e
}

/// The page's word for the roof's verdict.
fn roof_word(health: &hems_forecast::Health) -> String {
    let debug = format!("{health:?}").to_lowercase();
    if debug.starts_with("degraded") {
        "degraded".into()
    } else if debug.starts_with("learning") {
        "learning".into()
    } else {
        "healthy".into()
    }
}

/// A closed day, as a receipt.
#[must_use]
pub fn receipt_of(day: &hems_core::report::DayKpis) -> Receipt {
    let (cost, without) = day.economics.as_ref().map_or(
        (
            hems_core::prelude::CostBreakdown::default(),
            hems_core::prelude::CostBreakdown::default(),
        ),
        |e| (e.cost, e.baseline),
    );
    let total = |c: &hems_core::prelude::CostBreakdown| c.total();
    Receipt {
        day: day.date.to_string(),
        bill_eur: cost.energy_eur,
        wear_eur: cost.wear_eur,
        comfort_eur: cost.discomfort_eur,
        cost_eur: total(&cost),
        without_eur: total(&without),
        saved_eur: total(&without) - total(&cost),
        on_bill_eur: without.energy_eur - cost.energy_eur,
        imported_kwh: day.imported_kwh,
        produced_kwh: day.produced_kwh,
        reduced_minutes: None,
        reductions: day.control_events,
        reduction_respected: day.respected_the_grid,
        feed_in_respected: day.worst_feed_in_overshoot_w <= 0.0,
    }
}

/// RFC 3339, or an empty string.
fn rfc3339(at: time::OffsetDateTime) -> String {
    at.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// The devices of `site` a household would recognise, in a stable order.
#[must_use]
pub fn devices_of(site: &Site) -> Vec<(AssetId, &'static str)> {
    let order = [
        "car",
        "heat_pump",
        "battery",
        "hot_water",
        "appliance",
        "roof",
    ];
    let mut out: Vec<(AssetId, &'static str)> = site
        .assets
        .iter()
        .filter_map(|a| kind_of(a).map(|k| (a.id().clone(), k)))
        .collect();
    out.sort_by_key(|(id, k)| {
        (
            order.iter().position(|o| o == k).unwrap_or(99),
            id.to_string(),
        )
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hems_core::setpoint::Reason;

    fn every_reason() -> Vec<Reason> {
        let mut out = vec![Reason::Cem];
        for rule in [
            GuardRule::Lpc,
            GuardRule::Lpp,
            GuardRule::Para9Cap,
            GuardRule::Failsafe,
            GuardRule::CircuitLimit,
            GuardRule::ContractLimit,
            GuardRule::Unbalance,
            GuardRule::DeviceLimit,
            GuardRule::BackupReserve,
        ] {
            out.push(Reason::guard(rule));
        }
        for u in [UserOverride::Boost, UserOverride::Pause, UserOverride::Away] {
            out.push(Reason::User(u));
        }
        for c in [
            RealtimeCause::SurplusTracking,
            RealtimeCause::SelfConsumption,
            RealtimeCause::PhaseBalance,
            RealtimeCause::RampLimit,
            RealtimeCause::Hysteresis,
            RealtimeCause::LocalControl,
            RealtimeCause::MaximumPowerPoint,
        ] {
            out.push(Reason::Realtime(c));
        }
        for c in [
            FallbackCause::NoPlan,
            FallbackCause::PlanStale,
            FallbackCause::DriverLost,
            FallbackCause::ClockUnsynchronised,
        ] {
            out.push(Reason::Fallback(c));
        }
        out
    }

    #[test]
    fn every_reason_has_a_key_the_page_is_told_about() {
        for reason in every_reason() {
            let e = explain(&reason, Power::from_kw(1.0), Some(15.0));
            assert!(ALL_KEYS.contains(&e.key), "{} is not in ALL_KEYS", e.key);
        }
        assert!(ALL_KEYS.contains(&idle().key));
    }

    #[test]
    fn the_page_has_a_sentence_for_every_key_in_both_languages() {
        // The page is compiled into the binary; a key it cannot render would show
        // a household an identifier.
        let page = crate::ui::APP_JS;
        for key in ALL_KEYS {
            let needle = format!("\"{key}\":");
            assert_eq!(
                page.matches(&needle).count(),
                2,
                "the page needs a German and an English sentence for `{key}`"
            );
        }
    }

    #[test]
    fn a_reduction_is_the_operators_and_says_which_rule() {
        let e = explain(&Reason::guard(GuardRule::Lpc), Power::from_kw(2.1), None);
        assert_eq!(e.cause, Cause::Operator);
        assert!(e.citation.is_some_and(|c| c.contains("§ 14a")));
        assert_eq!(e.params.get("kw").map(String::as_str), Some("2.1"));
    }

    #[test]
    fn the_plan_says_charge_wait_or_discharge_from_the_sign() {
        let plan = |kw| {
            explain(
                &Reason::Plan {
                    plan: hems_core::prelude::PlanId::new(),
                    slot: Slot::containing(time::macros::datetime!(2026-01-15 03:30 UTC)),
                    marginal_eur_per_kwh: None,
                },
                Power::from_kw(kw),
                Some(15.0),
            )
            .key
        };
        assert_eq!(plan(11.0), "plan.charge");
        assert_eq!(plan(-3.0), "plan.discharge");
        assert_eq!(plan(0.0), "plan.wait");
    }

    fn session(
        now_kwh: f64,
        target_kwh: f64,
        slots: i64,
    ) -> (hems_optimizer::model::EvSession, Slot) {
        use hems_core::prelude::Energy;
        let now = Slot::containing(time::macros::datetime!(2026-01-14 21:00 UTC));
        let mut departure = now;
        for _ in 0..slots {
            departure = departure.next();
        }
        (
            hems_optimizer::model::EvSession {
                energy_now: Energy::from_kwh(now_kwh),
                energy_target: Energy::from_kwh(target_kwh),
                capacity: Energy::from_kwh(60.0),
                max_charge: Power::from_kw(11.0),
                min_charge: Power::from_kw(4.2),
                efficiency: 0.92,
                arrival: None,
                departure,
            },
            now,
        )
    }

    #[test]
    fn a_goal_with_a_night_ahead_is_feasible_and_one_with_an_hour_is_not() {
        // 44 % → 80 % of 60 kWh is 21,6 kWh; ten hours at 11 kW is ample.
        let (s, now) = session(26.4, 48.0, 40);
        assert_eq!(judge_goal(&s, now).verdict, Verdict::Feasible);
        // One hour delivers about 10 kWh, so 80 % is out of reach — and the
        // answer says what is reachable instead of planning for less silently.
        let (s, now) = session(26.4, 48.0, 4);
        let a = judge_goal(&s, now);
        assert_eq!(a.verdict, Verdict::Impossible);
        let reach = a.reachable_soc.expect("a reachable level");
        assert!(reach > 0.44 && reach < 0.80, "{reach}");
    }

    #[test]
    fn the_verdict_measures_what_the_planner_measures() {
        // Below one the ratio *is* the planner's tightness; only above it, where
        // the planner clamps, does the verdict see further.
        for slots in [9, 12, 20, 40] {
            let (s, now) = session(26.4, 48.0, slots);
            assert!(
                (demand_ratio(&s, now) - s.tightness(now)).abs() < 1e-9,
                "{slots}"
            );
        }
    }

    #[test]
    fn a_goal_that_needs_nearly_all_the_time_left_is_tight() {
        // 2,3 hours at 11 kW × 0,92 ≈ 23,3 kWh deliverable against 21,6 needed.
        let (s, now) = session(26.4, 48.0, 9);
        assert_eq!(judge_goal(&s, now).verdict, Verdict::Tight);
    }

    #[test]
    fn seven_in_the_morning_is_seven_on_the_wall_even_when_the_clocks_change() {
        use time::macros::datetime;
        // Winter: 07:00 Berlin is 06:00 UTC.
        assert_eq!(
            next_wall_clock(420, datetime!(2026-01-14 21:00 UTC)),
            datetime!(2026-01-15 06:00 UTC)
        );
        // The night the clocks go forward: 07:00 CEST is 05:00 UTC.
        assert_eq!(
            next_wall_clock(420, datetime!(2026-03-28 22:00 UTC)),
            datetime!(2026-03-29 05:00 UTC)
        );
        // Already past today: tomorrow's.
        assert_eq!(
            next_wall_clock(420, datetime!(2026-01-15 08:00 UTC)),
            datetime!(2026-01-16 06:00 UTC)
        );
    }

    #[test]
    fn a_car_already_there_is_met() {
        let (s, now) = session(50.0, 48.0, 8);
        assert_eq!(judge_goal(&s, now).verdict, Verdict::Met);
    }
}
