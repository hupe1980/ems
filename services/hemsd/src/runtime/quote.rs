//! What a goal would cost, before it is set.
//!
//! Since evcc 0.318 a plan that explains itself is table stakes; what no energy
//! manager does is let a household question the plan before committing to it.
//! The box can: it keeps the inputs of its last solve, so a proposed goal is
//! priced by solving **the same problem** twice — with the goal in force now and
//! with the proposed one — and subtracting. The price on the slider is then the
//! plan's own difference, to the cent, not a second model of the house.
//!
//! The verdict is the page's own (`home::judge_goal`, `home::judge_tank`), so a
//! quote and the answer after *Set* can never disagree about whether a goal is
//! reachable.

use serde::Serialize;
use time::OffsetDateTime;

use crate::home::{CarGoal, TankGoal, Verdict};
use crate::runtime::planner::{Inputs, LastInputs};

/// What a proposed goal comes to.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Quote {
    /// Met, feasible, tight or impossible — the page's own answer.
    pub verdict: Verdict,
    /// What is reachable by then where the goal is not: a state of charge in
    /// `[0, 1]` for a car, °C for a tank.
    pub reachable: Option<f64>,
    /// What the plan with this goal costs beyond the plan with the goal in
    /// force now, € — energy, battery wear and what is borrowed from the
    /// stores. Negative where it is cheaper. `None` where no plan could be made.
    pub extra_eur: Option<f64>,
    /// Confirmed quarter hours within a proposed car price cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eligible_quarters: Option<usize>,
    /// Absolute expiry of a priced immediate-charge request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boost_until: Option<i64>,
}

/// At most one quote in flight, and one a second.
///
/// A quote is two solves; on a box with two cores a page that quoted on every
/// tick of a slider, or a script that quoted in a loop, would starve the
/// planner the control loop depends on. The guard is process-wide because a
/// box serves one household.
static LAST_QUOTE: tokio::sync::Mutex<Option<std::time::Instant>> =
    tokio::sync::Mutex::const_new(None);

/// The shortest gap between two quotes.
pub const QUOTE_GAP: std::time::Duration = std::time::Duration::from_secs(1);

/// Hold the one quote slot, or `None` where another quote is running or the
/// last ended less than [`QUOTE_GAP`] ago.
pub fn slot() -> Option<tokio::sync::MutexGuard<'static, Option<std::time::Instant>>> {
    let held = LAST_QUOTE.try_lock().ok()?;
    if held.is_some_and(|at| at.elapsed() < QUOTE_GAP) {
        return None;
    }
    Some(held)
}

/// What the plan spends on the things a goal moves: the grid, the battery's
/// life, and what it borrows from the stores and would have to put back.
fn spend(solved: &ems_optimizer::solve::Solved) -> Option<f64> {
    let c = solved.plan.expected_cost.as_ref()?;
    Some(c.energy_eur + c.wear_eur + c.stored_eur)
}

/// Solve `inputs` as planned and as proposed, off the runtime, and subtract.
async fn difference(base: Inputs, proposed: Inputs) -> Option<f64> {
    tokio::task::spawn_blocking(move || {
        let now = spend(&base.solve().ok()?)?;
        let then = spend(&proposed.solve().ok()?)?;
        // Whole cents: what the page shows, and what a test can hold.
        Some(((then - now) * 100.0).round() / 100.0)
    })
    .await
    .ok()
    .flatten()
}

/// Price a car goal. `None` where the planner has not planned yet or sees no
/// car.
pub async fn car(
    last: &LastInputs,
    facts: &crate::home::CarFacts,
    goal: CarGoal,
    now: OffsetDateTime,
) -> Option<Quote> {
    let inputs = last.lock().await.clone()?;
    let full = (*facts.lock().await)?;
    let proposed = crate::home::session_for(&full, goal, now);
    let answer = crate::home::judge_car_price(
        &proposed,
        ems_core::prelude::Slot::containing(now),
        goal.price_cap_ct,
        Some(inputs.prices()),
    );
    let extra_eur = difference(
        inputs.clone(),
        inputs
            .with_car(Some(proposed))
            .with_price_cap(goal.price_cap_ct),
    )
    .await;
    Some(Quote {
        verdict: answer.verdict,
        reachable: answer.reachable_soc,
        eligible_quarters: answer.eligible_quarters,
        boost_until: None,
        extra_eur,
    })
}

/// Compare the saved departure with the earliest attainable departure. Price
/// only alternatives that deliver the full target and start in this quarter.
pub async fn charge_now(
    last: &LastInputs,
    full: ems_optimizer::model::EvSession,
    goal: CarGoal,
    now: OffsetDateTime,
) -> Option<Quote> {
    use ems_core::prelude::Slot;
    let inputs = last.lock().await.clone()?;
    let first = Slot::containing(now);
    if Slot::containing(inputs.at()) != first {
        return None;
    }
    let session = crate::home::session_for(&full, goal, now);
    let needed = (session.energy_target - session.energy_now).kwh();
    if needed <= 0.0 {
        return None;
    }
    let mut remaining = needed;
    let mut draw = Vec::new();
    let mut slot = first;
    while slot < session.departure {
        let price = inputs.prices().at(slot);
        if price.is_some_and(|p| p.price_known)
            && goal
                .price_cap_ct
                .is_none_or(|c| inputs.prices().within_cap(slot, c))
        {
            let duration = (slot.next().start() - slot.start().max(now)).as_seconds_f64() / 3600.0;
            let stored = remaining.min(session.max_charge.kw() * session.efficiency * duration);
            draw.push(ems_core::prelude::Power::from_kw(
                stored / (0.25 * session.efficiency),
            ));
            remaining -= stored;
        } else {
            return None;
        }
        slot = slot.next();
        if remaining <= 0.0 {
            break;
        }
    }
    if remaining > 0.0 {
        return None;
    }
    let early = ems_optimizer::model::EvSession {
        // Fixed averages include the partial first and final charging quarters.
        min_charge: ems_core::prelude::Power::ZERO,
        departure: slot,
        ..session
    };
    let base = inputs
        .with_car(Some(session))
        .with_price_cap(goal.price_cap_ct);
    let proposed = base.with_car(Some(early));
    tokio::task::spawn_blocking(move || {
        let later = base.solve().ok()?;
        let sooner = proposed.solve_with_ev_draw(&draw).ok()?;
        if later.unmet_charge.kwh() > 0.001
            || sooner.unmet_charge.kwh() > 0.001
            || sooner.flows.first()?.ev_charge.kw() <= 0.0
        {
            return None;
        }
        Some(Quote {
            verdict: Verdict::Feasible,
            reachable: None,
            eligible_quarters: None,
            boost_until: None,
            extra_eur: Some(((spend(&sooner)? - spend(&later)?) * 100.0).round() / 100.0),
        })
    })
    .await
    .ok()
    .flatten()
}

/// Price a tank goal. `None` where the planner has not planned yet or has no
/// measured tank.
pub async fn tank(
    last: &LastInputs,
    facts: &crate::home::TankFacts,
    tank: &ems_core::asset::DhwTank,
    goal: TankGoal,
    now: OffsetDateTime,
) -> Option<Quote> {
    let inputs = last.lock().await.clone()?;
    let model = (*facts.lock().await)?;
    let answer = crate::home::judge_tank(tank, &model, goal, now);
    let target = crate::home::tank_target(tank, goal, now);
    let extra_eur = difference(inputs.clone(), inputs.with_tank(Some(target))).await;
    Some(Quote {
        verdict: answer.verdict,
        reachable: answer.reachable_c,
        eligible_quarters: None,
        boost_until: None,
        extra_eur,
    })
}
