//! § 42c EnWG — Energy Sharing, the quarter-hourly allocation.
//!
//! Since **1 June 2026** final customers inside the Bilanzierungsgebiet of one
//! distribution network may use renewable electricity together *over the public
//! grid* (§ 42c Abs. 1 EnWG). What is shared is not physics — the electrons go
//! where they go — but an **allocation**: for each quarter hour, the community's
//! generation is divided among its consumers according to an
//! **Aufteilungsschlüssel** agreed in writing (§ 42c Abs. 3 Nr. 2), and the part
//! each member is allocated is billed at the community's price instead of at
//! their supplier's.
//!
//! Two things follow, and this module is both of them:
//!
//! * **Eligibility** — a delivery point may only take part if both its
//!   consumption and its generation are measured by Zählerstandsgangmessung
//!   *or* by quarter-hourly registrierende Leistungsmessung (§ 42c Abs. 1).
//!   That decision belongs to metering and lives in [`metering::allocation::sharing`]; hems
//!   consumes it rather than restating it.
//! * **Allocation** — the arithmetic below.
//!
//! # A static key and a dynamic one are two different contracts
//!
//! Applying a key once and capping each member at what they used leaves
//! generation on whoever happened to be away: a member allocated 3 kWh who
//! consumed 1 kWh cannot take the other two, and those two are then simply not
//! shared. Re-offering them to whoever still has unmet consumption shares more
//! — but it is a *different allocation*, and § 42c Abs. 3 Nr. 2 makes the
//! Aufteilungsschlüssel a written agreement between the parties.
//!
//! So it is a choice the community makes and hems records, not one hems makes
//! for it. [`Aufteilung::Statisch`] applies the key once and reports the
//! remainder as unallocated — generation that went to the public grid and is
//! settled the ordinary way. [`Aufteilung::Dynamisch`] cascades: the key is
//! applied, each member capped at what they actually consumed, and the
//! remainder re-offered to whoever still has unmet consumption in the same
//! proportions, until nothing is left to give or nobody is left to take it.
//! Both are defensible; picking one silently is not.
//!
//! # Whose arithmetic
//!
//! All of it is [`metering::allocation::community::allocate`] — one
//! implementation of "divide a quantity by a key and cap each part", with the
//! identity `Σ allocated + residual = total` holding exactly on every row. The
//! static key is metering's `Constant` Aufteilungsfaktor (six decimal places,
//! rounded down so the factors never sum above one), the dynamic one its
//! `Cascading` key. hems contributes the community, the slot and the choice;
//! the arithmetic and its conservation are metering's, the same § 42b is
//! settled with.
//!
//! Every number here is billed, so it is [`rust_decimal::Decimal`]: a
//! settlement that went through an `f64` is one nobody can reproduce. The
//! planner's `f64` view of the same community is a
//! forecast of what *could* be shared; this is what *was*.

use core::fmt;

use ems_core::prelude::Slot;
use std::collections::BTreeMap;

use metering::allocation::community::{AllocationKey, allocate as allocate_series};
use metering::allocation::formula::SplitFactor;
use metering::{DayBoundary, MeloId, MeterInterval, QualityFlag, Resolution, Series};
use rust_decimal::Decimal;
use thiserror::Error;
use time::Date;

/// Which allocation the community agreed in writing, § 42c Abs. 3 Nr. 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Aufteilung {
    /// Apply the key once; whatever a member cannot use stays unallocated.
    ///
    /// The literal reading of a fixed Aufteilungsschlüssel. It under-shares by
    /// construction — a member who is away strands their share — and that is
    /// what the parties agreed to if this is what they wrote down.
    Statisch,
    /// Re-offer what a member cannot use to whoever still has unmet
    /// consumption, in the same proportions, until the generation or the demand
    /// runs out.
    ///
    /// The default, because it is what an energy-sharing community is *for* and
    /// what a key expressed as proportions rather than fixed quantities
    /// naturally means. It shares strictly more than [`Statisch`](Self::Statisch)
    /// and never gives anybody more than they used.
    #[default]
    Dynamisch,
}

/// The day § 42c allocation became a duty of the network operator.
pub const SHARING_START: Date = time::macros::date!(2026 - 06 - 01);

/// One participant of an energy-sharing community.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Member {
    /// The Marktlokation this member is billed at.
    pub malo: String,
    /// The member's share of the Aufteilungsschlüssel, § 42c Abs. 3 Nr. 2.
    ///
    /// Any non-negative number: the shares are normalised, so 1/2/3 and
    /// 10/20/30 mean the same thing and a community can express its key in
    /// whatever units its contract uses.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub key: Decimal,
}

impl Member {
    /// A member with a share of the key.
    #[must_use]
    pub fn new(malo: impl Into<String>, key: Decimal) -> Self {
        Self {
            malo: malo.into(),
            key,
        }
    }
}

/// A sharing community, as § 42c Abs. 3 and 4 describe one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Community {
    /// The Bilanzierungsgebiet the community lives in, § 42c Abs. 4 Nr. 1.
    ///
    /// Until 1 June 2028 this is the boundary: a member outside it cannot take
    /// part, however close by they are.
    pub bilanzierungsgebiet: String,
    /// The consumers, with their shares of the key.
    pub members: Vec<Member>,
}

impl Community {
    /// A community with a key.
    #[must_use]
    pub fn new(bilanzierungsgebiet: impl Into<String>, members: Vec<Member>) -> Self {
        Self {
            bilanzierungsgebiet: bilanzierungsgebiet.into(),
            members,
        }
    }

    /// The sum of the key's shares.
    #[must_use]
    pub fn key_total(&self) -> Decimal {
        self.members.iter().map(|m| m.key).sum()
    }
}

/// Why an allocation could not be produced.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SharingError {
    /// A community with no members has nothing to allocate to.
    #[error("the community has no members")]
    NoMembers,
    /// Every share is zero, so the key says nothing.
    ///
    /// Splitting evenly instead would be inventing a contract the parties did
    /// not sign, which is exactly the kind of guess a settlement may not make.
    #[error("every share of the Aufteilungsschlüssel is zero")]
    EmptyKey,
    /// A negative share, a negative generation or a negative consumption.
    #[error("{0} is negative")]
    Negative(&'static str),
    /// The consumption vector does not match the community's membership.
    #[error("expected {expected} consumption values, got {actual}")]
    LengthMismatch {
        /// How many members the community has.
        expected: usize,
        /// How many values were supplied.
        actual: usize,
    },
    /// `metering` refused the interval — a value outside `Decimal`'s range,
    /// or a slot that is not a quarter hour.
    #[error("metering refused the allocation: {0}")]
    Allocation(String),
}

/// What one member was allocated in one quarter hour.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Share {
    /// Which member.
    pub malo: String,
    /// What they consumed in the quarter hour, kWh.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub consumption: Decimal,
    /// What was allocated to them from the community's generation, kWh.
    ///
    /// Never more than [`Share::consumption`]: a member cannot be sold shared
    /// electricity they did not use.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub shared: Decimal,
    /// What is left for their ordinary supplier to deliver, kWh.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub residual: Decimal,
}

/// The allocation of one quarter hour, § 42c Abs. 3.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Allocation {
    /// The quarter hour.
    pub slot: Slot,
    /// The community's renewable generation offered in it, kWh.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub generation: Decimal,
    /// One entry per member, in the community's own order.
    pub shares: Vec<Share>,
    /// Generation the community could not place, because every member's demand
    /// was already met, kWh.
    ///
    /// It is fed into the grid as ordinary production and settled with the
    /// Direktvermarkter or under the EEG — the community simply did not use it.
    /// A community whose surplus is persistently large has a key that does not
    /// match its load, and this is the number that says so.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub unallocated: Decimal,
}

impl Allocation {
    /// Everything the community shared in this quarter hour, kWh.
    #[must_use]
    pub fn shared_total(&self) -> Decimal {
        self.shares.iter().map(|s| s.shared).sum()
    }

    /// Everything the community consumed in this quarter hour, kWh.
    #[must_use]
    pub fn consumption_total(&self) -> Decimal {
        self.shares.iter().map(|s| s.consumption).sum()
    }

    /// The share of the community's consumption that came from its own
    /// generation — the number a community is actually trying to raise.
    #[must_use]
    pub fn coverage(&self) -> Decimal {
        let total = self.consumption_total();
        if total.is_zero() {
            Decimal::ZERO
        } else {
            self.shared_total() / total
        }
    }
}

impl fmt::Display for Allocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} kWh shared of {} kWh generated, {} kWh unallocated",
            self.slot,
            self.shared_total(),
            self.generation,
            self.unallocated
        )
    }
}

/// Allocate one quarter hour's generation over a community, § 42c Abs. 3.
///
/// [`allocate_by`] under [`Aufteilung::Dynamisch`].
///
/// # Errors
/// As [`allocate_by`].
pub fn allocate(
    community: &Community,
    slot: Slot,
    generation: Decimal,
    consumption: &[Decimal],
) -> Result<Allocation, SharingError> {
    allocate_by(
        community,
        slot,
        generation,
        consumption,
        Aufteilung::default(),
    )
}

/// Allocate one quarter hour's generation over a community under a named
/// Aufteilung, § 42c Abs. 3.
///
/// `consumption` is one value per member of `community`, in the same order.
/// The arithmetic is `metering`'s, so `Σ shared + unallocated` equals the
/// generation exactly, whichever [`Aufteilung`] is used.
///
/// # Errors
/// [`SharingError`] when the community is empty, when the key is all zeros, when
/// a value is negative, or when the consumption vector is the wrong length.
pub fn allocate_by(
    community: &Community,
    slot: Slot,
    generation: Decimal,
    consumption: &[Decimal],
    aufteilung: Aufteilung,
) -> Result<Allocation, SharingError> {
    if community.members.is_empty() {
        return Err(SharingError::NoMembers);
    }
    if consumption.len() != community.members.len() {
        return Err(SharingError::LengthMismatch {
            expected: community.members.len(),
            actual: consumption.len(),
        });
    }
    if generation < Decimal::ZERO {
        return Err(SharingError::Negative("generation"));
    }
    if consumption.iter().any(|c| *c < Decimal::ZERO) {
        return Err(SharingError::Negative("consumption"));
    }
    if community.members.iter().any(|m| m.key < Decimal::ZERO) {
        return Err(SharingError::Negative(
            "a share of the Aufteilungsschlüssel",
        ));
    }
    if community.key_total().is_zero() {
        return Err(SharingError::EmptyKey);
    }

    let n = community.members.len();
    let total = community.key_total();
    let refused = |e: &dyn fmt::Display| SharingError::Allocation(e.to_string());
    let one = |kwh: Decimal| -> Result<Series, SharingError> {
        let interval = MeterInterval::quarter_hour(slot.start(), kwh, QualityFlag::Measured)
            .map_err(|e| refused(&e))?;
        Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, vec![interval])
            .map_err(|e| refused(&e))
    };

    // The key, in `metering`'s terms. A member with no share of it is no
    // participant: under either Aufteilung it is credited nothing.
    let melo = |i: usize| -> MeloId {
        format!("DE{i:031}")
            .parse()
            .expect("a 33-character DE identifier is a MeloId")
    };
    let key = match aufteilung {
        // The key as written, applied once. `metering`'s Aufteilungsfaktor has
        // six decimal places, so each fraction is rounded **down** to six:
        // their sum stays at most one, and what the rounding leaves over stays
        // unallocated rather than being credited to nobody in particular.
        Aufteilung::Statisch => AllocationKey::Constant {
            fractions: (0..n)
                .filter_map(|i| {
                    let fraction = (community.members[i].key / total)
                        .round_dp_with_strategy(6, rust_decimal::RoundingStrategy::ToZero);
                    SplitFactor::new(fraction).ok().map(|f| (melo(i), f))
                })
                .collect::<BTreeMap<_, _>>(),
        },
        // What a member cannot use is re-offered to whoever still has unmet
        // consumption, in the key's proportions, until nothing moves —
        // `metering`'s cascading key, exact in `Decimal`.
        Aufteilung::Dynamisch => AllocationKey::Cascading {
            weights: (0..n)
                .filter(|&i| community.members[i].key > Decimal::ZERO)
                .map(|i| (melo(i), community.members[i].key))
                .collect(),
        },
    };
    let in_key = |i: usize| match &key {
        AllocationKey::Constant { fractions } => fractions.contains_key(&melo(i)),
        AllocationKey::Cascading { weights } => weights.contains_key(&melo(i)),
        _ => false,
    };
    let members: Vec<usize> = (0..n).filter(|&i| in_key(i)).collect();
    let series = members
        .iter()
        .map(|&i| one(consumption[i]))
        .collect::<Result<Vec<_>, _>>()?;
    let participants: Vec<(MeloId, &Series)> =
        members.iter().map(|&i| melo(i)).zip(&series).collect();
    let rows = allocate_series(&one(generation)?, &participants, &key).map_err(|e| refused(&e))?;
    let row = rows
        .first()
        .ok_or_else(|| SharingError::Allocation("no interval was allocated".into()))?;

    let mut shared = vec![Decimal::ZERO; n];
    for &i in &members {
        if let Some(share) = row.share(&melo(i)) {
            shared[i] = share.allocated;
        }
    }
    let remaining = row.residual;

    let allocated = community
        .members
        .iter()
        .zip(consumption)
        .zip(&shared)
        .map(|((member, used), got)| Share {
            malo: member.malo.clone(),
            consumption: *used,
            shared: *got,
            residual: (*used - *got).max(Decimal::ZERO),
        })
        .collect();

    Ok(Allocation {
        slot,
        generation,
        shares: allocated,
        unallocated: remaining.max(Decimal::ZERO),
    })
}

/// Whether a network operator must already make § 42c allocation possible on
/// `day`.
///
/// # Where the refusal belongs, and where it does not
///
/// **Not in [`allocate_by`].** The arithmetic is defined by the community's own
/// contract (§ 42c Abs. 3), not by the date, and a caller that wants the
/// conservation identity checked over a day's registers is asking a question
/// about arithmetic rather than making a claim about money. A function that
/// refused a date would be a rule stated over the wrong noun — the same mistake
/// as banning a crate when the invariant is about a feature.
///
/// **At the boundary where an allocation becomes a claim**, which is the shape
/// this workspace already uses for the sibling rule: `histd`'s MiSpeL export
/// refuses a period the Festlegung never reached, and the error lives at the
/// export rather than in the quantity. What § 42c Abs. 4 Nr. 1 fixes is the day
/// a *network operator* must make sharing possible, so before it a household
/// may agree whatever it likes and will not be allocated anything — and a box
/// that planned against a share, or a report that credited one, would be
/// promising money that will not arrive.
///
/// It was called by nothing for as long as it existed, which is how two
/// reference days came to settle a § 42c allocation on dates in January and May
/// 2026 and report the credit (D179).
#[must_use]
pub fn applies_on(day: Date) -> bool {
    day >= SHARING_START
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::prelude::FromPrimitive;

    fn dec(v: f64) -> Decimal {
        Decimal::from_f64(v).unwrap().round_dp(9)
    }

    fn slot() -> Slot {
        Slot::containing(time::macros::datetime!(2026-06-01 12:00:00 UTC))
    }

    fn community() -> Community {
        Community::new(
            "11XG-BILANZ-DE-1",
            vec![
                Member::new("DE0001", Decimal::ONE),
                Member::new("DE0002", Decimal::ONE),
                Member::new("DE0003", Decimal::TWO),
            ],
        )
    }

    #[test]
    fn a_static_key_strands_what_a_member_away_cannot_use() {
        // The two Aufteilungen on one quarter hour, and the whole difference
        // between them. 12 kWh generated, an even three-way key, and one member
        // who used nothing.
        let community = Community::new(
            "BG-1",
            vec![
                Member::new("A", Decimal::ONE),
                Member::new("B", Decimal::ONE),
                Member::new("C", Decimal::ONE),
            ],
        );
        let slot = Slot::containing(time::macros::datetime!(2026-07-01 10:00 UTC));
        let used = [dec(6.0), dec(6.0), Decimal::ZERO];

        let statisch =
            allocate_by(&community, slot, dec(12.0), &used, Aufteilung::Statisch).unwrap();
        assert_eq!(statisch.shares[2].shared, Decimal::ZERO);
        // Four kilowatt-hours went to the public grid, to a millionth: a
        // proportional share of a third is cut to six places before anything is
        // subtracted, which is `metering`'s guarantee that the identity below
        // holds rather than a rounding error on top of it.
        assert!(
            (statisch.unallocated - dec(4.0)).abs() < dec(0.00001),
            "got {}",
            statisch.unallocated
        );
        assert_eq!(statisch.shared_total() + statisch.unallocated, dec(12.0));

        let dynamisch =
            allocate_by(&community, slot, dec(12.0), &used, Aufteilung::Dynamisch).unwrap();
        assert!(dynamisch.unallocated < dec(0.00001));
        assert!((dynamisch.shares[0].shared - dec(6.0)).abs() < dec(0.00001));
        assert!((dynamisch.shares[1].shared - dec(6.0)).abs() < dec(0.00001));
        assert_eq!(dynamisch.shared_total() + dynamisch.unallocated, dec(12.0));
        assert!(
            dynamisch.shared_total() > statisch.shared_total(),
            "a dynamic key shares strictly more"
        );
    }

    #[test]
    fn every_allocation_conserves_the_generation_exactly() {
        // `metering`'s identity, which is why each pass goes through it: shares
        // are cut to a millionth of a kilowatt-hour *before* anything is
        // subtracted, so the residual is a difference rather than an
        // accumulation and a year of quarter hours cannot drift.
        let community = Community::new(
            "BG-2",
            vec![
                Member::new("A", Decimal::ONE),
                Member::new("B", Decimal::ONE),
                Member::new("C", Decimal::ONE),
            ],
        );
        let slot = Slot::containing(time::macros::datetime!(2026-07-01 10:00 UTC));
        for aufteilung in [Aufteilung::Statisch, Aufteilung::Dynamisch] {
            let a = allocate_by(
                &community,
                slot,
                Decimal::ONE,
                &[dec(1.0), dec(1.0), dec(1.0)],
                aufteilung,
            )
            .unwrap();
            assert_eq!(
                a.shared_total() + a.unallocated,
                Decimal::ONE,
                "{aufteilung:?} lost a millionth of a kilowatt-hour"
            );
        }
    }

    #[test]
    fn generation_that_fits_is_split_by_the_key() {
        // 8 kWh over a 1:1:2 key is 2, 2, 4 — and everybody can use their share.
        let a = allocate(
            &community(),
            slot(),
            dec(8.0),
            &[dec(5.0), dec(5.0), dec(5.0)],
        )
        .unwrap();
        assert_eq!(a.shares[0].shared, dec(2.0));
        assert_eq!(a.shares[1].shared, dec(2.0));
        assert_eq!(a.shares[2].shared, dec(4.0));
        assert_eq!(a.unallocated, Decimal::ZERO);
        assert_eq!(a.shares[2].residual, dec(1.0));
    }

    #[test]
    fn a_member_who_is_away_does_not_strand_the_communitys_generation() {
        // The reason for the cascade. The first member consumed nothing, so the
        // 2 kWh the key would have given them is re-offered to the other two in
        // *their* proportions — 1:2 — and every kilowatt-hour is placed.
        let a = allocate(
            &community(),
            slot(),
            dec(8.0),
            &[Decimal::ZERO, dec(5.0), dec(5.0)],
        )
        .unwrap();
        assert_eq!(a.shares[0].shared, Decimal::ZERO);
        assert_eq!(a.shared_total(), dec(8.0));
        assert_eq!(a.unallocated, Decimal::ZERO);
        // 8 kWh over the remaining 1:2 key would be 2,67 and 5,33 — but the
        // third member only wants 5, so the fourth of a kilowatt-hour that
        // leaves goes back to the second.
        assert_eq!(a.shares[2].shared, dec(5.0));
        assert_eq!(a.shares[1].shared, dec(3.0));
    }

    #[test]
    fn nobody_is_allocated_more_than_they_used() {
        let a = allocate(
            &community(),
            slot(),
            dec(40.0),
            &[dec(1.0), dec(2.0), dec(3.0)],
        )
        .unwrap();
        for s in &a.shares {
            assert!(s.shared <= s.consumption, "{s:?}");
            assert_eq!(s.residual, Decimal::ZERO);
        }
        assert_eq!(a.shared_total(), dec(6.0));
        assert_eq!(a.unallocated, dec(34.0), "the rest is ordinary feed-in");
    }

    #[test]
    fn the_allocation_never_exceeds_the_generation_and_never_loses_any() {
        // The invariant a network operator will check first: what came in went
        // out, exactly, with no rounding drift — which is why this is `Decimal`
        // and not `f64`.
        for generation in [0.0, 0.5, 3.0, 6.0, 6.000_1, 100.0] {
            let a = allocate(
                &community(),
                slot(),
                dec(generation),
                &[dec(1.0), dec(2.0), dec(3.0)],
            )
            .unwrap();
            assert_eq!(
                a.shared_total() + a.unallocated,
                dec(generation),
                "generation {generation}"
            );
        }
    }

    #[test]
    fn a_key_of_all_zeros_is_refused_rather_than_split_evenly() {
        // Splitting evenly would be inventing a contract the parties did not
        // sign. § 42c Abs. 3 Nr. 2 makes the key a written agreement; an absent
        // one is a missing document, not a default.
        let flat = Community::new(
            "11XG-BILANZ-DE-1",
            vec![
                Member::new("DE0001", Decimal::ZERO),
                Member::new("DE0002", Decimal::ZERO),
            ],
        );
        assert_eq!(
            allocate(&flat, slot(), dec(5.0), &[dec(1.0), dec(1.0)]),
            Err(SharingError::EmptyKey)
        );
    }

    #[test]
    fn a_member_with_no_share_of_the_key_gets_nothing() {
        let c = Community::new(
            "11XG-BILANZ-DE-1",
            vec![
                Member::new("DE0001", Decimal::ZERO),
                Member::new("DE0002", Decimal::ONE),
            ],
        );
        let a = allocate(&c, slot(), dec(4.0), &[dec(5.0), dec(5.0)]).unwrap();
        assert_eq!(a.shares[0].shared, Decimal::ZERO);
        assert_eq!(a.shares[1].shared, dec(4.0));
    }

    #[test]
    fn the_wrong_number_of_readings_is_a_mismatch_and_not_a_zero() {
        assert_eq!(
            allocate(&community(), slot(), dec(1.0), &[dec(1.0)]),
            Err(SharingError::LengthMismatch {
                expected: 3,
                actual: 1
            })
        );
    }

    #[test]
    fn coverage_is_what_the_community_is_trying_to_raise() {
        let a = allocate(
            &community(),
            slot(),
            dec(3.0),
            &[dec(2.0), dec(2.0), dec(2.0)],
        )
        .unwrap();
        assert_eq!(a.coverage(), dec(0.5), "3 kWh shared of 6 kWh consumed");
    }

    #[test]
    fn nothing_generated_shares_nothing_and_is_not_an_error() {
        let a = allocate(
            &community(),
            slot(),
            Decimal::ZERO,
            &[dec(2.0), dec(2.0), dec(2.0)],
        )
        .unwrap();
        assert_eq!(a.shared_total(), Decimal::ZERO);
        for s in &a.shares {
            assert_eq!(s.residual, s.consumption);
        }
    }

    #[test]
    fn sharing_starts_on_the_first_of_june() {
        assert!(!applies_on(time::macros::date!(2026 - 05 - 31)));
        assert!(applies_on(SHARING_START));
    }

    /// The predicate that says when a network operator's duty begins, and the
    /// boundary it draws.
    ///
    /// It is only a predicate: [`allocate_by`] does **not** consult it, because
    /// the arithmetic is contract-defined and a caller checking a conservation
    /// identity over a day's registers is not claiming a household was
    /// allocated anything. The refusal belongs where an allocation becomes a
    /// claim, which is what D179 moved.
    #[test]
    fn the_duty_begins_on_the_first_of_june_and_not_before() {
        assert!(!applies_on(time::macros::date!(2026 - 05 - 31)));
        assert!(applies_on(SHARING_START));
        assert!(applies_on(time::macros::date!(2028 - 06 - 01)));

        // And the arithmetic answers whatever it is asked, on either side of
        // that boundary, because it is a different question.
        let community = Community::new(
            "11YDE-VE-------2",
            vec![
                Member::new("DE0001111111111111111111111111111", Decimal::ONE),
                Member::new("DE0002222222222222222222222222222", Decimal::ONE),
            ],
        );
        for at in [
            time::macros::datetime!(2026-05-31 12:00:00 UTC),
            time::macros::datetime!(2026-06-01 12:00:00 UTC),
        ] {
            let allocation = allocate(
                &community,
                Slot::containing(at),
                Decimal::from(4),
                &[Decimal::from(2), Decimal::from(2)],
            )
            .expect("the identity is a question about arithmetic");
            assert_eq!(
                allocation.shares.iter().map(|s| s.shared).sum::<Decimal>()
                    + allocation.unallocated,
                Decimal::from(4)
            );
        }
    }
}
