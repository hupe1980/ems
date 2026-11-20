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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
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
    "plan.wait_until",
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
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Receipt {
    /// The local day, `YYYY-MM-DD`.
    pub day: String,
    /// Whether the day was priced. A running box measures energy but has no
    /// second house to compare with; the page then prints no euros rather
    /// than zeros.
    pub priced: bool,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
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
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct CarGoal {
    /// The state of charge wanted, in `[0, 1]`.
    pub target_soc: f64,
    /// By when, local wall-clock minutes after midnight (`420` is 07:00).
    pub by_minute: u16,
}

/// A hot-water goal as the household set it: *50 °C by 18:00*.
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct TankGoal {
    /// The temperature wanted, °C.
    pub target_c: f64,
    /// By when, local wall-clock minutes after midnight (`1080` is 18:00).
    pub by_minute: u16,
}

/// The answer a household gets the moment it sets a hot-water goal.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct TankAnswer {
    /// The verdict.
    pub verdict: Verdict,
    /// Electricity the heater still has to take, kWh.
    pub needed_kwh: f64,
    /// The share of the time left the goal would use: `0` nothing, `1` all of it.
    pub tightness: f64,
    /// Where the goal is impossible: the temperature the tank can reach by
    /// then, °C.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reachable_c: Option<f64>,
}

/// The tank as the planner last saw it, published so a new goal can be judged
/// at once, without a solve.
pub type TankFacts = std::sync::Arc<tokio::sync::Mutex<Option<hems_optimizer::model::DhwModel>>>;

/// Judge a hot-water goal with the planner's own tank model.
///
/// `tank` is the installation (litres, the temperatures it may hold) and
/// `facts` the store the planner plans — heater, COP, standing loss, the heat in
/// it now. What the household draws before the time is its own business and is
/// not guessed at: the verdict says whether the *heater* can get there.
#[must_use]
pub fn judge_tank(
    tank: &hems_core::asset::DhwTank,
    facts: &hems_optimizer::model::DhwModel,
    goal: TankGoal,
    now: time::OffsetDateTime,
) -> TankAnswer {
    let per_kelvin = tank.heat_capacity_kwh_per_k();
    let cop = facts.cop.max(f64::EPSILON);
    let now_c = tank.t_min_c + facts.stored_now.kwh() / per_kelvin.max(f64::EPSILON);
    let heat = per_kelvin * (goal.target_c - now_c);
    if heat <= 0.0 {
        return TankAnswer {
            verdict: Verdict::Met,
            needed_kwh: 0.0,
            tightness: 0.0,
            reachable_c: None,
        };
    }
    #[allow(clippy::cast_precision_loss)]
    let hours = Slot::containing(now)
        .distance_to(Slot::containing(next_wall_clock(goal.by_minute, now)))
        .max(0) as f64
        * 0.25;
    // What the heater puts in less what the cylinder loses meanwhile.
    let deliverable = ((facts.heater.kw() * cop - facts.standing_loss.kw()) * hours).max(0.0);
    let tightness = if deliverable > 0.0 {
        heat / deliverable
    } else {
        f64::INFINITY
    };
    // Above the highest safe temperature no amount of time helps.
    let verdict = if tightness > 1.0 || goal.target_c > tank.t_max_c {
        Verdict::Impossible
    } else if tightness > TIGHT_ABOVE {
        Verdict::Tight
    } else {
        Verdict::Feasible
    };
    TankAnswer {
        verdict,
        needed_kwh: heat / cop,
        tightness,
        reachable_c: (verdict == Verdict::Impossible)
            .then(|| (now_c + deliverable / per_kelvin.max(f64::EPSILON)).min(tank.t_max_c)),
    }
}

/// The planner's target for a hot-water goal.
#[must_use]
pub fn tank_target(
    tank: &hems_core::asset::DhwTank,
    goal: TankGoal,
    now: time::OffsetDateTime,
) -> hems_optimizer::model::DhwTarget {
    hems_optimizer::model::DhwTarget {
        stored: tank.stored_heat(goal.target_c),
        by: Slot::containing(next_wall_clock(goal.by_minute, now)),
    }
}

/// What the household has asked for: at most one goal per kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GoalSet {
    /// The car.
    #[serde(default)]
    pub car: Option<CarGoal>,
    /// The hot-water tank.
    #[serde(default)]
    pub tank: Option<TankGoal>,
}

/// The household's goals, shared between the page and the planner, and kept in
/// the box's store where it has one — a goal set on Sunday still stands after
/// Monday's power cut.
#[derive(Clone, Default)]
pub struct Goals {
    set: std::sync::Arc<tokio::sync::Mutex<GoalSet>>,
    store: Option<std::sync::Arc<tokio::sync::Mutex<crate::store::Store>>>,
}

impl Goals {
    /// The goals `store` holds, kept there from now on.
    ///
    /// # Errors
    /// [`StoreError`](crate::store::StoreError) where the read fails.
    pub async fn remembering(
        store: Option<std::sync::Arc<tokio::sync::Mutex<crate::store::Store>>>,
    ) -> Result<Self, crate::store::StoreError> {
        let set = match &store {
            Some(store) => store.lock().await.goals::<GoalSet>()?.unwrap_or_default(),
            None => GoalSet::default(),
        };
        Ok(Self {
            set: std::sync::Arc::new(tokio::sync::Mutex::new(set)),
            store,
        })
    }

    /// The goals in force.
    pub async fn get(&self) -> GoalSet {
        *self.set.lock().await
    }

    /// Change them, and keep the change. Answers whether anything changed.
    ///
    /// # Errors
    /// [`StoreError`](crate::store::StoreError) where the store refuses the
    /// write; the goals in force are then unchanged.
    pub async fn change(
        &self,
        change: impl FnOnce(&mut GoalSet),
    ) -> Result<bool, crate::store::StoreError> {
        let mut set = self.set.lock().await;
        let mut next = *set;
        change(&mut next);
        if next == *set {
            return Ok(false);
        }
        if let Some(store) = &self.store {
            store.lock().await.put_goals(&next)?;
        }
        *set = next;
        Ok(true)
    }
}

/// A goal as `PUT /v1/goals/{asset}` takes it: the body's shape follows the
/// asset's kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NewGoal {
    /// For a charge point.
    Car(CarGoal),
    /// For a hot-water tank.
    Tank(TankGoal),
}

/// Why a goal was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalRefused {
    /// The site has no such asset.
    UnknownAsset,
    /// The asset is neither a charge point nor a hot-water tank.
    NoGoalForThisKind,
    /// The body is not a goal for this kind of asset — a temperature for a car.
    WrongShape,
    /// The values are outside what a goal can be.
    OutOfRange,
}

impl GoalRefused {
    /// What to tell the caller.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::UnknownAsset => "no such device",
            Self::NoGoalForThisKind => "goals are for a charge point or a hot-water tank",
            Self::WrongShape => {
                "a charge point takes target_soc and by_minute; a hot-water tank takes \
                 target_c and by_minute"
            }
            Self::OutOfRange => {
                "a goal is a state of charge in [0, 1] or a temperature in (0, 100] °C, \
                 and a time of day in minutes below 1440"
            }
        }
    }
}

/// Which goal `asset` takes: `car`, `hot_water`, or a refusal.
///
/// # Errors
/// [`GoalRefused::UnknownAsset`], [`GoalRefused::NoGoalForThisKind`].
pub fn goal_kind(site: &Site, asset: &str) -> Result<&'static str, GoalRefused> {
    let found = site
        .assets
        .iter()
        .find(|a| a.id().to_string() == asset)
        .ok_or(GoalRefused::UnknownAsset)?;
    match kind_of(found) {
        Some(kind @ ("car" | "hot_water")) => Ok(kind),
        _ => Err(GoalRefused::NoGoalForThisKind),
    }
}

/// Read a goal for `asset` out of a request body, typed by the asset's kind.
///
/// # Errors
/// [`GoalRefused`], for an asset that takes no goal, a body of the wrong kind,
/// or values no goal can have.
pub fn parse_goal(
    site: &Site,
    asset: &str,
    body: serde_json::Value,
) -> Result<NewGoal, GoalRefused> {
    let goal = match goal_kind(site, asset)? {
        "car" => NewGoal::Car(serde_json::from_value(body).map_err(|_| GoalRefused::WrongShape)?),
        _ => NewGoal::Tank(serde_json::from_value(body).map_err(|_| GoalRefused::WrongShape)?),
    };
    let in_range = match goal {
        NewGoal::Car(g) => (0.0..=1.0).contains(&g.target_soc) && g.by_minute < 1440,
        NewGoal::Tank(g) => g.target_c > 0.0 && g.target_c <= 100.0 && g.by_minute < 1440,
    };
    if in_range {
        Ok(goal)
    } else {
        Err(GoalRefused::OutOfRange)
    }
}

/// What the plan in force expects the hours ahead to put on the bill.
///
/// One figure, and deliberately not two. The plan also prices the same horizon
/// without an energy manager, and since 0.4 that baseline is sound — but the
/// *difference* between the two over a rolling horizon is not a saving a
/// household can be shown. A plan hedges against the dull day and pays the
/// premium in the median future this figure describes; it values what it
/// leaves in the battery and the walls, which no bill credits; and its horizon
/// opens and closes with the stores in different states on the two sides. On
/// the January reference day 42 of 96 plans cost more than their own baseline
/// on paper while the closed day saved €1,07. The saving is the receipt's,
/// where the day is over and the ledger is closed.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Ahead {
    /// The first instant covered, RFC 3339.
    pub from: String,
    /// The instant the window ends, RFC 3339.
    pub until: String,
    /// What the plan expects the window to put on the electricity bill, €:
    /// imports less what feeding in earns. Negative on a day that earns.
    pub bill_eur: f64,
}

/// The bill the plan expects for its horizon.
///
/// `None` — and the page then shows nothing — where the plan carries no cost,
/// where it is not a number, or where the plan has run out: a wrong figure
/// here is worse than none.
#[must_use]
pub fn ahead(plan: &hems_core::prelude::Plan, now: time::OffsetDateTime) -> Option<Ahead> {
    let bill = plan.expected_cost?.billed_eur();
    let until = plan.horizon.end();
    let from = plan.horizon.first.start();
    if !bill.is_finite() || now >= until || now < from {
        return None;
    }
    Some(Ahead {
        from: rfc3339(from),
        until: rfc3339(until),
        bill_eur: bill,
    })
}

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
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
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
    /// What the plan expects its horizon to put on the bill. Absent where the
    /// plan carries no price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ahead: Option<Ahead>,
    /// The day so far.
    pub receipt: Option<Receipt>,
    /// The car goal in force, and what it comes to.
    pub goal: Option<(CarGoal, GoalAnswer)>,
    /// The hot-water goal in force, and what it comes to.
    pub tank_goal: Option<(TankGoal, TankAnswer)>,
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
    /// The year whose published levies the box is pricing on, **only** while
    /// that is not the current year: the new year's figures are not in this
    /// version of the box, and every price it shows is on last year's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levies_year: Option<i32>,
}

/// The year of the published levies in use on `now`, where it is not `now`'s.
#[must_use]
pub fn stale_levies_year(now: time::OffsetDateTime) -> Option<i32> {
    let year = Slot::containing(now).local_date().year();
    let (published, _) = hems_tariff::Levies::latest_published(year);
    (published != year).then_some(published)
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
    /// The household's goals.
    pub goals: GoalSet,
    /// The car as the planner last saw it.
    pub car: Option<hems_optimizer::model::EvSession>,
    /// The tank as the planner last saw it.
    pub tank: Option<hems_optimizer::model::DhwModel>,
    /// The box's SKI and certificate fingerprint.
    pub ski: Option<String>,
    /// The SHA-256 fingerprint of its certificate.
    pub fingerprint: Option<String>,
    /// The SHA-256 fingerprint of the certificate it serves HTTPS with.
    pub https_fingerprint: Option<String>,
}

/// The household page's view of a running box.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one document, assembled field by field in the order the page reads it"
)]
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

    let devices = devices_now(input, price_ct, &horizon_prices, &|id| {
        next_use(input.plan?, id, slot, &price_of)
    });

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

    let tank_goal = input.goals.tank.and_then(|g| {
        let tank = input.site.assets.iter().find_map(|a| match a {
            Asset::Dhw(tank) => Some(tank),
            _ => None,
        })?;
        Some((g, judge_tank(tank, input.tank.as_ref()?, g, now)))
    });
    let goal = input.goals.car.map(|g| {
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
        ahead: input.plan.and_then(|p| ahead(p, now)),
        receipt: status.last_day.as_ref().map(receipt_of),
        goal,
        tank_goal,
        ski: input.ski.clone(),
        fingerprint: input.fingerprint.clone(),
        https_fingerprint: input.https_fingerprint.clone(),
        roof: status.roof_health.as_ref().map(roof_word),
        levies_year: stale_levies_year(now),
    }
}

/// Every device the household would recognise, with what it is doing and why.
fn devices_now(
    input: &Live<'_>,
    price_ct: Option<f64>,
    horizon_prices: &[f64],
    next: &dyn Fn(&AssetId) -> Option<(String, f64)>,
) -> Vec<Device> {
    let status = input.status;
    devices_of(input.site)
        .into_iter()
        .map(|(id, kind)| {
            let (measured, soc) = status.readings.get(&id).copied().unwrap_or((None, None));
            let commanded = status.commanded.get(&id).copied();
            let shown = measured.or(commanded);
            let why = waiting(
                status.reasons.get(&id).map_or_else(idle, |reason| {
                    relative(
                        explain(reason, commanded.unwrap_or(Power::ZERO), price_ct),
                        price_ct,
                        horizon_prices,
                    )
                }),
                next(&id),
            );
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

/// When the plan next runs `asset`, and at what price: the local wall-clock
/// time of the first later quarter hour it draws in, and that slot's ct/kWh.
fn next_use(
    plan: &hems_core::prelude::Plan,
    asset: &AssetId,
    now: Slot,
    price_of: &dyn Fn(Slot) -> Option<f64>,
) -> Option<(String, f64)> {
    let slot = plan
        .slots
        .iter()
        .filter(|s| s.slot > now)
        .find(|s| {
            s.targets
                .iter()
                .any(|t| t.asset == *asset && t.power.kw() > 0.05)
        })?
        .slot;
    let local = hems_core::calendar::to_berlin(slot.start());
    Some((
        format!("{:02}:{:02}", local.hour(), local.minute()),
        price_of(slot)?,
    ))
}

/// A device that is waiting says what for.
///
/// "Off" and "waiting for a cheaper time" are true and tell a household
/// nothing it can check. Where the plan in force runs the device later, the
/// explanation names the time and the price it is waiting for.
fn waiting(mut e: Explanation, next: Option<(String, f64)>) -> Explanation {
    if !matches!(e.key, "idle" | "plan.wait") {
        return e;
    }
    if let Some((at, then_ct)) = next {
        e.key = "plan.wait_until";
        e.cause = Cause::Plan;
        e.params.insert("at", at);
        e.params.insert("then_ct", format!("{then_ct:.0}"));
    }
    e
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
        priced: day.economics.is_some(),
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

    fn reference_tank() -> (hems_core::asset::DhwTank, hems_optimizer::model::DhwModel) {
        let site = crate::site::Household::build(&crate::HouseholdConfig::default())
            .expect("the reference household")
            .site;
        let tank = site
            .assets
            .iter()
            .find_map(|a| match a {
                Asset::Dhw(t) => Some(t.clone()),
                _ => None,
            })
            .expect("the reference tank");
        let facts = hems_optimizer::model::DhwModel {
            // Lukewarm: 48 °C in a tank that holds 45 to 60.
            stored_now: tank.stored_heat(48.0),
            cop: tank.cop,
            standing_loss: tank.standing_loss,
            ..hems_optimizer::model::DhwModel::tank(tank.usable_heat(), tank.heater)
        };
        (tank, facts)
    }

    #[test]
    fn hot_water_by_six_is_feasible_and_seventy_degrees_never_is() {
        use time::macros::datetime;
        let (tank, facts) = reference_tank();
        let noon = datetime!(2026-01-15 11:00 UTC);
        let goal = |target_c| TankGoal {
            target_c,
            by_minute: 18 * 60,
        };
        let fifty = judge_tank(&tank, &facts, goal(50.0), noon);
        assert_eq!(fifty.verdict, Verdict::Feasible, "{fifty:?}");
        assert!(fifty.needed_kwh > 0.0 && fifty.reachable_c.is_none());

        // Above the highest safe temperature no heater gets there, and the
        // answer names what the tank *can* hold.
        let seventy = judge_tank(&tank, &facts, goal(70.0), noon);
        assert_eq!(seventy.verdict, Verdict::Impossible);
        assert_eq!(seventy.reachable_c, Some(tank.t_max_c));

        // Already warmer than asked.
        assert_eq!(
            judge_tank(&tank, &facts, goal(46.0), noon).verdict,
            Verdict::Met
        );

        // A quarter of an hour is not enough for ten kelvin in 300 litres, and
        // the reachable temperature lies between now and the goal.
        let late = judge_tank(&tank, &facts, goal(58.0), datetime!(2026-01-15 16:45 UTC));
        assert_eq!(late.verdict, Verdict::Impossible, "{late:?}");
        let reach = late.reachable_c.expect("a reachable temperature");
        assert!(reach > 48.0 && reach < 58.0, "{reach}");
    }

    #[test]
    fn a_goal_is_typed_by_the_asset_it_is_for() {
        let site = crate::site::Household::build(&crate::HouseholdConfig::default())
            .expect("the reference household")
            .site;
        let id = |kind: &str| {
            devices_of(&site)
                .into_iter()
                .find(|(_, k)| *k == kind)
                .map(|(id, _)| id.to_string())
                .expect("the reference household has one")
        };
        let car = serde_json::json!({ "target_soc": 0.8, "by_minute": 420 });
        let water = serde_json::json!({ "target_c": 50.0, "by_minute": 1080 });

        assert!(matches!(
            parse_goal(&site, &id("car"), car.clone()),
            Ok(NewGoal::Car(_))
        ));
        assert!(matches!(
            parse_goal(&site, &id("hot_water"), water.clone()),
            Ok(NewGoal::Tank(_))
        ));
        // A temperature for a car, a state of charge for a tank.
        assert_eq!(
            parse_goal(&site, &id("car"), water),
            Err(GoalRefused::WrongShape)
        );
        assert_eq!(
            parse_goal(&site, &id("hot_water"), car.clone()),
            Err(GoalRefused::WrongShape)
        );
        assert_eq!(
            parse_goal(&site, &id("battery"), car.clone()),
            Err(GoalRefused::NoGoalForThisKind)
        );
        assert_eq!(
            parse_goal(&site, "no-such-device", car),
            Err(GoalRefused::UnknownAsset)
        );
        assert_eq!(
            parse_goal(
                &site,
                &id("hot_water"),
                serde_json::json!({ "target_c": 50.0, "by_minute": 1440 })
            ),
            Err(GoalRefused::OutOfRange)
        );
    }

    #[tokio::test]
    async fn goals_survive_a_restart_and_not_a_factory_reset() {
        let store = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::store::Store::in_memory().expect("a store"),
        ));
        let goals = Goals::remembering(Some(store.clone()))
            .await
            .expect("an empty store reads as no goals");
        assert_eq!(goals.get().await, GoalSet::default());
        let asked = TankGoal {
            target_c: 50.0,
            by_minute: 1080,
        };
        assert!(goals.change(|g| g.tank = Some(asked)).await.expect("kept"));
        assert!(
            !goals.change(|g| g.tank = Some(asked)).await.expect("kept"),
            "the same goal again changes nothing"
        );

        // A restart: a new holder over the same store.
        let again = Goals::remembering(Some(store.clone()))
            .await
            .expect("the goals read back");
        assert_eq!(
            again.get().await,
            GoalSet {
                car: None,
                tank: Some(asked)
            }
        );

        store
            .lock()
            .await
            .factory_reset_discarding_evidence()
            .expect("a reset");
        let reset = Goals::remembering(Some(store)).await.expect("readable");
        assert_eq!(reset.get().await, GoalSet::default());
    }

    #[test]
    fn a_waiting_device_says_what_it_waits_for() {
        let e = waiting(idle(), Some(("01:15".into(), 28.4)));
        assert_eq!(e.key, "plan.wait_until");
        assert_eq!(e.params.get("at").map(String::as_str), Some("01:15"));
        assert_eq!(e.params.get("then_ct").map(String::as_str), Some("28"));
        // Nothing planned: it is simply off.
        assert_eq!(waiting(idle(), None).key, "idle");
        // A device that is running is not waiting, whatever the plan does later.
        let running = explain(
            &Reason::guard(GuardRule::Lpc),
            Power::from_kw(2.0),
            Some(30.0),
        );
        assert_eq!(
            waiting(running, Some(("01:15".into(), 28.0))).key,
            "guard.lpc"
        );
    }

    #[test]
    fn the_bill_ahead_is_shown_only_where_the_plan_prices_it() {
        use hems_core::prelude::{CostBreakdown, Horizon, Plan};
        let now = time::macros::datetime!(2026-01-15 16:30 UTC);
        let mut plan = Plan::empty(Horizon::new(now, 96), now);
        assert_eq!(ahead(&plan, now), None, "a plan with no cost shows none");
        plan.expected_cost = Some(CostBreakdown {
            energy_eur: 4.0,
            wear_eur: 0.12,
            ..CostBreakdown::default()
        });
        let a = ahead(&plan, now).expect("priced");
        // The bill, not the bill with wear in it: what the invoice will say.
        assert!((a.bill_eur - 4.0).abs() < 1e-9);
        assert_eq!(a.from, "2026-01-15T16:30:00Z");
        assert_eq!(a.until, "2026-01-16T16:30:00Z");
        // A plan that has run out describes hours that are over.
        assert_eq!(ahead(&plan, now + time::Duration::hours(25)), None);
        plan.expected_cost = Some(CostBreakdown::energy_only(f64::NAN));
        assert_eq!(ahead(&plan, now), None, "not a number is not a price");
    }

    #[test]
    fn a_car_already_there_is_met() {
        let (s, now) = session(50.0, 48.0, 8);
        assert_eq!(judge_goal(&s, now).verdict, Verdict::Met);
    }
}
