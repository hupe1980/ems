//! MiSpeL — Marktintegration von Speichern und Ladepunkten, BNetzA Az. 618-25-02.
//!
//! A household battery that has ever been charged from the grid, or a
//! bidirectional charge point, keeps its levy privileges (§ 21 EnFG) and its
//! market premium (§ 19 EEG) only for the energy the Festlegung attributes to
//! it — green or grey, quarter hour by quarter hour, summed over a calendar
//! month (Anlage 1) or a calendar year (Anlage 2). Get it wrong and the
//! household loses the levy privilege on its grid draw, the premium on its
//! feed-in, or both.
//!
//! | Option | Who it is for | Here |
//! |---|---|---|
//! | **Ausschließlichkeit** | a store that is never charged from the grid | nothing to compute — `Battery::grid_charging_allowed == false` |
//! | **Abgrenzung** (Anlage 1) | anything else | [`abgrenzung_month`], formulas (1)–(32), Basisfälle A1–A4 |
//! | **Pauschal** (Anlage 2) | solar ≤ 30 kWp, one meter | [`pauschal_period`], formulas (P1)–(P16), Basisfälle P1–P3 |
//!
//! # Whose arithmetic
//!
//! **`metering`'s.** Every formula is `metering::eeg::mispel`, written against
//! the adopted text and held to its worked examples; this module keeps what is
//! hems's — the register record a box writes ([`QuarterHour`]), the refusal of
//! a register no meter could have produced, the rule set a result was computed
//! under, and the window the rules govern ([`governed`]). A second copy of the
//! formulas here was a second answer to one bill, and it had already drifted:
//! it followed the Arbeitsstand of 05.08.2026, whose (P2)P2 was 0,5 where the
//! adopted Anlage 2 says 0,2.
//!
//! Citations are `[MiSpeL A1 (n)]` for formula `n` of Anlage 1 and
//! `[MiSpeL A2 (Pn)]` for Anlage 2 (BNetzA, Az. 618-25-02, Festlegung of
//! 01.10.2026, Anlage 1 *Abgrenzungsoption* and Anlage 2 *Pauschaloption*),
//! and `[MiSpeL Tenor n]` for a Tenorziffer (Amtsblatt 19/2026, Vfg. Nr. 73/2026).
//!
//! # When the rules govern
//!
//! `[MiSpeL Tenor 8]` makes the rules take effect with the Bekanntgabe — two
//! weeks after the Amtsblatt of 07.10.2026, so **21.10.2026**. `[MiSpeL Tenor 9]`
//! adds two conditions: until 30.09.2027 only with the consent of the network
//! and metering operators, and the Pauschaloption only from the first day of the
//! month after the European Commission's state-aid approval. And an installation
//! already assigned to a Veräußerungsform switches only on the first day of a
//! month (Anlage 1 § 11, Anlage 2 § 9, § 21b Abs. 1 S. 2 EEG by analogy), so a
//! partial month or year — a *Rumpfmonat*, a *Rumpfjahr* — arises only from a
//! first assignment. [`governed`] turns all of that into the one window a
//! settlement may cover.
//!
//! Every number here becomes money or a Nachweis, so it is
//! [`rust_decimal::Decimal`] throughout.

use core::fmt;
use std::collections::BTreeSet;

use ems_core::prelude::Slot;
use metering::eeg::EegError;
use metering::eeg::mispel::geltung::{self as mg, Optionsart, Umstaende};
use metering::eeg::mispel::{Einspeisung, Gate, abgrenzung as ma, pauschal as mp};
use metering::{DayBoundary, MeterInterval, QualityFlag, Resolution, Series};
use rust_decimal::Decimal;
use thiserror::Error;
use time::{Date, OffsetDateTime};

/// The storage round-trip efficiency the Festlegung *presumes* wherever a charge
/// point makes a measured one impossible, `[MiSpeL A1 (14)]` for A2–A4.
pub const PRESUMED_EFFICIENCY: Decimal = ma::WIRKUNGSGRAD_LADEPUNKT;

/// The yearly feed-in per kilowatt of installed solar power the Pauschaloption
/// treats as supportable, `[MiSpeL A2 (P1)]`.
pub const PAUSCHAL_KWH_PER_KW: Decimal = mp::FOERDERGRENZE_KWH_JE_KWP;

/// The largest solar installation the Pauschaloption is open to, kWp
/// (§ 19 Abs. 3c EEG).
pub const PAUSCHAL_MAX_KWP: Decimal = Decimal::from_parts(30, 0, 0, false, 0);

// ── The rule set ─────────────────────────────────────────────────────────────

/// Which version of the Festlegung a result was computed under.
///
/// A Nachweis that cannot name the rules that produced it is not evidence of
/// anything, so the version travels with every result — and a new Festlegung is
/// a new variant here, which makes every place that has to think about the
/// difference fail to compile until it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
// The lint fires because `Decimal` has `unsafe` methods; nothing about
// deserialising a fieldless enum can be.
#[allow(clippy::unsafe_derive_deserialize)]
#[non_exhaustive]
pub enum RuleSet {
    /// BNetzA 618-25-02, decided 01.10.2026, in effect from 21.10.2026
    /// (`[MiSpeL Tenor 8]`).
    #[default]
    Festlegung20261001,
}

impl RuleSet {
    /// The document this version was read from.
    #[must_use]
    pub const fn version(self) -> &'static str {
        match self {
            RuleSet::Festlegung20261001 => "BNetzA 618-25-02, Festlegung 01.10.2026",
        }
    }

    /// The day these rules take effect, `[MiSpeL Tenor 8]` — `metering`'s
    /// [`mg::WIRKSAM_AB`], held there to the Amtsblatt notice.
    #[must_use]
    pub const fn effective_from(self) -> Date {
        match self {
            RuleSet::Festlegung20261001 => mg::WIRKSAM_AB,
        }
    }

    /// Whether these rules govern `day`.
    #[must_use]
    pub fn applies_on(self, day: Date) -> bool {
        day >= self.effective_from()
    }

    /// The last day on which the rules apply only with the network and
    /// metering operators' consent, `[MiSpeL Tenor 9]` a).
    #[must_use]
    pub const fn consent_required_until(self) -> Option<Date> {
        match self {
            RuleSet::Festlegung20261001 => Some(mg::EINVERSTAENDNIS_BIS),
        }
    }

    /// The first day the Pauschaloption applies, given the day the European
    /// Commission approved § 19 Abs. 3c EEG — the first day of the following
    /// month, `[MiSpeL Tenor 9]` b). `None` while there is no approval.
    #[must_use]
    pub fn pauschal_applicable_from(self, approved_on: Option<Date>) -> Option<Date> {
        // `metering`'s `geltung`, asked with the consent condition met so that
        // only Tenorziffer 8 and 9 b decide: never before the Festlegung itself
        // took effect, and refused rather than dated without an approval.
        match self {
            RuleSet::Festlegung20261001 => mg::geltung(
                Optionsart::Pauschal,
                Umstaende {
                    einverstaendnis: true,
                    genehmigung: Some(approved_on?),
                },
            )
            .ok()
            .map(|g| g.ab),
        }
    }
}

impl fmt::Display for RuleSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.version())
    }
}

// ── The governed window ─────────────────────────────────────────────────────

/// The option a window is computed for: what unit it settles over, and whether
/// the state-aid gate applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MispelOption {
    /// Anlage 1 — per calendar month.
    Abgrenzung,
    /// Anlage 2 — per calendar year, from the state-aid month.
    Pauschal,
}

/// The assignment of the installation to the option, `[MiSpeL Tenor 5]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assignment {
    /// The first day the installation is assigned on the option's basis.
    pub from: Date,
    /// Whether this is the **first** assignment of a newly commissioned plant.
    /// Only then may it start mid-month; a switch of a plant that was already
    /// assigned to a Veräußerungsform happens on the first of a month.
    pub first: bool,
}

impl Assignment {
    /// Whether the text permits this assignment at all.
    ///
    /// # Errors
    /// [`Refusal::AssignedBeforeTheRules`] before the rules took effect,
    /// [`Refusal::MidMonthSwitch`] for a switch that is not on the first of a
    /// month (Anlage 1 § 11, § 21b Abs. 1 S. 2 EEG by analogy).
    pub fn validate(self, rules: RuleSet) -> Result<(), Refusal> {
        let effective_from = rules.effective_from();
        if self.from < effective_from {
            return Err(Refusal::AssignedBeforeTheRules {
                assigned_from: self.from,
                effective_from,
            });
        }
        if !self.first && self.from.day() != 1 {
            return Err(Refusal::MidMonthSwitch {
                assigned_from: self.from,
            });
        }
        Ok(())
    }
}

/// The facts the window depends on that are declared rather than measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conditions {
    /// The assignment.
    pub assignment: Assignment,
    /// Whether the network and metering operators consented, `[MiSpeL Tenor 9]` a).
    pub operator_consent: bool,
    /// The day the European Commission approved § 19 Abs. 3c EEG, if it has.
    pub state_aid_approved_on: Option<Date>,
}

/// The part of a period the rules govern, inclusive at both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Governed {
    /// The first governed day.
    pub first: Date,
    /// The last governed day.
    pub last: Date,
    /// Whether the window is shorter than the period — a Rumpfmonat (Anlage 1
    /// § 11) or Rumpfjahr (Anlage 2 § 9).
    pub rumpf: bool,
}

/// Why a period settles under no rules at all.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum Refusal {
    /// The period ends before the rules took effect.
    #[error(
        "the period ends before the Festlegung took effect on {effective_from} \
         (Tenorziffer 8: effect with the Bekanntgabe)"
    )]
    BeforeTheRules {
        /// The effect date.
        effective_from: Date,
    },
    /// The period ends before the installation was assigned.
    #[error(
        "the period ends before the installation was assigned to the option on {assigned_from}"
    )]
    BeforeTheAssignment {
        /// The assignment date.
        assigned_from: Date,
    },
    /// An assignment dated before the rules took effect.
    #[error(
        "an assignment on {assigned_from} is before the Festlegung took effect on {effective_from}"
    )]
    AssignedBeforeTheRules {
        /// The assignment date.
        assigned_from: Date,
        /// The effect date.
        effective_from: Date,
    },
    /// A switch of an existing plant that does not start on the first of a month.
    #[error(
        "an installation already assigned to a Veräußerungsform switches only on the first \
         day of a month (§ 21b Abs. 1 S. 2 EEG by analogy), not on {assigned_from}; declare \
         `first_assignment` only for a newly commissioned plant"
    )]
    MidMonthSwitch {
        /// The assignment date.
        assigned_from: Date,
    },
    /// Before the consent deadline without consent.
    #[error(
        "until {until} the Festlegung applies only with the consent of the network and \
         metering operators (Tenorziffer 9 a), and none is declared"
    )]
    NoConsent {
        /// The last day consent is required.
        until: Date,
    },
    /// The Pauschaloption before any state-aid approval.
    #[error(
        "the Pauschaloption applies only from the month after the European Commission's \
         state-aid approval of § 19 Abs. 3c EEG (Tenorziffer 9 b), and none is configured"
    )]
    NoStateAid,
    /// The Pauschaloption before the month after approval.
    #[error("the Pauschaloption applies from {applicable_from} (Tenorziffer 9 b)")]
    BeforeStateAid {
        /// The first applicable day.
        applicable_from: Date,
    },
}

/// The part of `[period_first, period_last]` the rules govern for one site.
///
/// The window starts at the latest of: the period's own start, the effect date
/// (`[MiSpeL Tenor 8]`), the assignment, the day after the consent deadline
/// where no consent is declared (`[MiSpeL Tenor 9]` a), and — for the
/// Pauschaloption — the first day of the month after state-aid approval
/// (`[MiSpeL Tenor 9]` b). A start inside the period makes it a Rumpf period,
/// which the Anlagen settle *"an die Stelle des Kalendermonats"* / *"…des
/// Kalenderjahres"*.
///
/// # Errors
/// A [`Refusal`] naming the condition that leaves nothing of the period
/// governed.
pub fn governed(
    rules: RuleSet,
    option: MispelOption,
    conditions: Conditions,
    period_first: Date,
    period_last: Date,
) -> Result<Governed, Refusal> {
    let effective_from = rules.effective_from();
    if effective_from > period_last {
        return Err(Refusal::BeforeTheRules { effective_from });
    }
    let mut first = period_first.max(effective_from);

    let assigned_from = conditions.assignment.from;
    if assigned_from > period_last {
        return Err(Refusal::BeforeTheAssignment { assigned_from });
    }
    first = first.max(assigned_from);

    if !conditions.operator_consent
        && let Some(until) = rules.consent_required_until()
    {
        if until >= period_last {
            return Err(Refusal::NoConsent { until });
        }
        first = first.max(until.next_day().unwrap_or(until));
    }

    if option == MispelOption::Pauschal {
        let applicable_from = rules
            .pauschal_applicable_from(conditions.state_aid_approved_on)
            .ok_or(Refusal::NoStateAid)?;
        if applicable_from > period_last {
            return Err(Refusal::BeforeStateAid { applicable_from });
        }
        first = first.max(applicable_from);
    }

    Ok(Governed {
        first,
        last: period_last,
        rumpf: first > period_first,
    })
}

// ── The register record ─────────────────────────────────────────────────────

/// Which Basisfall of Anlage 1 the installation's metering concept matches,
/// `[MiSpeL A1 4.1]`.
///
/// The case is a property of the **meters**, not of the devices: it says which
/// of `Z2` and `Z3` exist and therefore which formulas can be evaluated at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Basisfall {
    /// A1 — a storage system, no charge point. The round-trip efficiency can be
    /// *measured*, which is the only case in which it is.
    A1,
    /// A2 — a bidirectional charge point, no storage system.
    A2,
    /// A3 — both, on one meter. The simplified alternative to A4.
    A3,
    /// A4 — both, with the storage system separately metered at `Z3`, which is
    /// what makes its losses privilegeable.
    A4,
}

impl Basisfall {
    /// Whether the round-trip efficiency is measured (A1) or presumed at 85 %
    /// (everything with a charge point in it), `[MiSpeL A1 (14)]`.
    #[must_use]
    pub const fn efficiency_is_measured(self) -> bool {
        matches!(self, Basisfall::A1)
    }

    /// Whether storage losses can be privileged at all, `[MiSpeL A1 (19)]`.
    #[must_use]
    pub const fn losses_are_privilegeable(self) -> bool {
        matches!(self, Basisfall::A1 | Basisfall::A4)
    }

    /// Whether the case needs the separate storage meter `Z3`.
    #[must_use]
    pub const fn needs_z3(self) -> bool {
        matches!(self, Basisfall::A4)
    }
}

impl fmt::Display for Basisfall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Basisfall::A1 => "A1",
            Basisfall::A2 => "A2",
            Basisfall::A3 => "A3",
            Basisfall::A4 => "A4",
        })
    }
}

/// One quarter hour of measured energy, in kilowatt-hours.
///
/// The names are the Festlegung's own — `Z1NB¼`, `Z1NE¼`, `Z2V¼`, `Z2E¼`,
/// `Z3V¼`, `Z3E¼` — because a Nachweis that renames its inputs is a Nachweis
/// somebody has to translate before they can check it.
///
/// Every quantity is a **non-negative magnitude**: these are register
/// differences from real meters, not the signed control values of `ems-core`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QuarterHour {
    /// The quarter hour.
    pub slot: Slot,
    /// `Z1NB¼` — grid draw at the Entnahmestelle.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub grid_draw: Decimal,
    /// `Z1NE¼` — grid feed-in at the Einspeisestelle.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub grid_feed_in: Decimal,
    /// `Z2V¼` — consumption of the storage system and/or charge point.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub device_consumption: Decimal,
    /// `Z2E¼` — generation from the storage system and/or charge point.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub device_generation: Decimal,
    /// `Z3V¼` — consumption of the storage system alone. Only case A4.
    #[cfg_attr(feature = "serde", serde(default))]
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str_option"))]
    pub storage_consumption: Option<Decimal>,
    /// `Z3E¼` — generation from the storage system alone. Only case A4.
    #[cfg_attr(feature = "serde", serde(default))]
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str_option"))]
    pub storage_generation: Option<Decimal>,
    /// `AW¼` — the anzulegender Wert of the generating plant in this quarter
    /// hour, ct/kWh. Zero in the hours § 51 EEG switches support off.
    #[cfg_attr(
        feature = "serde",
        serde(default, with = "rust_decimal::serde::str_option")
    )]
    pub anzulegender_wert: Option<Decimal>,
    /// `SP¼` — the spot price in this quarter hour, ct/kWh. Only the
    /// Pauschaloption reads it, `[MiSpeL A2 (P5)]`.
    #[cfg_attr(feature = "serde", serde(default))]
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str_option"))]
    pub spot_price: Option<Decimal>,
}

/// A producer's immutable revision of one settlement interval.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct RegisterAssertion {
    /// Source revision time, stable across retries, with microsecond precision.
    #[cfg_attr(feature = "serde", serde(with = "time::serde::rfc3339"))]
    pub recorded_at: OffsetDateTime,
    /// The complete register set at this revision.
    pub registers: QuarterHour,
}

impl QuarterHour {
    /// A quarter hour with nothing in it — the starting point for a builder-ish
    /// construction in tests and in a driver that fills in what it has.
    #[must_use]
    pub fn empty(slot: Slot) -> Self {
        Self {
            slot,
            grid_draw: Decimal::ZERO,
            grid_feed_in: Decimal::ZERO,
            device_consumption: Decimal::ZERO,
            device_generation: Decimal::ZERO,
            storage_consumption: None,
            storage_generation: None,
            anzulegender_wert: None,
            spot_price: None,
        }
    }

    /// `(1)¼ = MIN[Z1NB¼ ; Z2V¼]` — the grid electricity the store took in this
    /// quarter hour, `[MiSpeL A1 (1)]`.
    ///
    /// This single `MIN` is the **Speichervorrang** (`[MiSpeL A1 2.1.5]`): grid
    /// draw is attributed to the store before it is attributed to anything else.
    /// The planner reads it to refuse a simultaneous charge, and the
    /// Ausschließlichkeit check reads it to prove none happened.
    #[must_use]
    pub fn simultaneous_grid_charging(&self) -> Decimal {
        self.grid_draw.min(self.device_consumption)
    }

    /// `(2)¼ = MIN[Z1NE¼ ; Z2E¼]` — the feed-in that came out of the store in
    /// this quarter hour, `[MiSpeL A1 (2)]`.
    #[must_use]
    pub fn simultaneous_device_feed_in(&self) -> Decimal {
        self.grid_feed_in.min(self.device_generation)
    }

    /// `(24)¼ = WENN[AW¼ > 0 ; 1 ; 0]` — whether support is payable at all in
    /// this quarter hour, `[MiSpeL A1 (24)]`.
    #[must_use]
    pub fn supported(&self) -> Option<bool> {
        self.anzulegender_wert.map(|aw| aw > Decimal::ZERO)
    }

    /// `(P5)¼ = WENN[SP¼ ≥ 0 ; 1 ; 0]`, `[MiSpeL A2 (P5)]`.
    #[must_use]
    pub fn spot_non_negative(&self) -> Option<bool> {
        self.spot_price.map(|spot| spot >= Decimal::ZERO)
    }
}

/// Why a set of quarter hours could not be evaluated.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum MispelError {
    /// No quarter hours at all. An empty month is not a zero month; it is a
    /// missing measurement, and the difference matters to a Nachweis.
    #[error("no quarter-hour values were supplied")]
    NoValues,
    /// A negative register difference. Meters count upwards; a negative
    /// difference is a reading error or a sign convention mix-up, and both are
    /// worth failing on rather than silently clamping.
    #[error("quarter hour {slot} has a negative value for {field}")]
    NegativeValue {
        /// Which quarter hour.
        slot: Slot,
        /// Which quantity.
        field: &'static str,
    },
    /// The applicable support value could not be established for exported energy.
    #[error("unknown support value for feed-in at {0}")]
    MissingSupportValue(Slot),
    /// Feed-in occurred and the published spot price needed by (P5) is unknown.
    #[error("no published spot price for feed-in at {0}")]
    MissingSpotPrice(Slot),
    /// Case A4 needs the separate storage meter, `[MiSpeL A1 3.2.4]`.
    #[error("Basisfall A4 requires the separately metered storage values Z3V/Z3E")]
    MissingStorageMeter,
    /// The Pauschaloption is open only to solar up to 30 kWp.
    #[error("the Pauschaloption needs a solar installation of at most 30 kWp, got {0} kWp")]
    TooLargeForPauschal(Decimal),
    /// A storage case of the Pauschaloption without a storage capacity.
    #[error("Pauschalfall {0:?} needs the installed storage capacity SKinst")]
    NoStorageCapacity(PauschalFall),
    /// A quarter hour outside the month or year being settled.
    #[error("the quarter hour starting {0} lies outside the settled period")]
    OutsidePeriod(OffsetDateTime),
    /// `metering` refused the evaluation for another reason.
    #[error("the settlement arithmetic refused the registers: {0}")]
    Metering(String),
}

impl From<EegError> for MispelError {
    fn from(e: EegError) -> Self {
        match e {
            EegError::Empty => MispelError::NoValues,
            EegError::OutsidePeriod { at } => MispelError::OutsidePeriod(at),
            other => MispelError::Metering(other.to_string()),
        }
    }
}

/// A register's Festlegung name and how to read it off a quarter hour.
type Register = (&'static str, fn(&QuarterHour) -> Decimal);

fn validate(quarters: &[QuarterHour], fields: &[Register]) -> Result<(), MispelError> {
    if quarters.is_empty() {
        return Err(MispelError::NoValues);
    }
    for q in quarters {
        for (field, value) in fields {
            if value(q) < Decimal::ZERO {
                return Err(MispelError::NegativeValue {
                    slot: q.slot,
                    field,
                });
            }
        }
    }
    Ok(())
}

/// One register as a `metering` series on the settlement grid.
fn series(
    quarters: &[QuarterHour],
    value: impl Fn(&QuarterHour) -> Decimal,
) -> Result<Series, MispelError> {
    let intervals = quarters
        .iter()
        .map(|q| {
            MeterInterval::quarter_hour(q.slot.start(), value(q), QualityFlag::Measured)
                .map_err(|e| MispelError::Metering(e.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, intervals)
        .map_err(|e| MispelError::Metering(e.to_string()))
}

fn supported_quarters(quarters: &[QuarterHour]) -> Result<BTreeSet<OffsetDateTime>, MispelError> {
    if let Some(missing) = quarters
        .iter()
        .find(|q| q.grid_feed_in > Decimal::ZERO && q.anzulegender_wert.is_none())
    {
        return Err(MispelError::MissingSupportValue(missing.slot));
    }
    Ok(gate(quarters, |q| q.supported() == Some(true)))
}

fn gate(
    quarters: &[QuarterHour],
    passes: impl Fn(&QuarterHour) -> bool,
) -> BTreeSet<OffsetDateTime> {
    quarters
        .iter()
        .filter(|q| passes(q))
        .map(|q| q.slot.start())
        .collect()
}

// ── Anlage 1 — the Abgrenzungsoption ─────────────────────────────────────────

/// The Abgrenzungsoption for one calendar month or Rumpfmonat,
/// `[MiSpeL A1 4.2]`.
///
/// Every intermediate figure is kept, numbered as in the Festlegung, because a
/// Nachweis is checked by somebody re-deriving it. A share whose denominator is
/// zero is **undefined** (`None`), not zero: the Festlegung defines no value
/// for it, and the figures it feeds are zero by their own formulas.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Abgrenzung {
    /// Which metering concept this was computed under.
    pub fall: Basisfall,
    /// Which version of the Festlegung.
    pub rules: RuleSet,
    /// `(3)` total grid draw.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub grid_draw: Decimal,
    /// `(4)` total grid feed-in.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub grid_feed_in: Decimal,
    /// `(5)` total consumption of storage and/or charge point.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub device_consumption: Decimal,
    /// `(6)` total generation of storage and/or charge point.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub device_generation: Decimal,
    /// `(9)` grid electricity that went into the store.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub grid_charged: Decimal,
    /// `(10)` plant electricity that went into the store.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub plant_charged: Decimal,
    /// `(11)` feed-in out of the store, quarter hour by quarter hour.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub device_feed_in_base: Decimal,
    /// `(12)` **Fremdtankstrom**: energy a car brought from somebody else's
    /// charge point, which may be neither settled nor supported here.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub foreign_charge: Decimal,
    /// `(13)` feed-in out of the store that may be considered at all.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub device_feed_in_considered: Decimal,
    /// `(14)` round-trip efficiency — measured `(6)/(5)` in A1, presumed at 0,85
    /// otherwise; undefined in an A1 month the store took nothing.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str_option"))]
    pub efficiency: Option<Decimal>,
    /// `(15)` the green share of what came out of the store.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub renewable_storage_output: Decimal,
    /// `(16)` **saldierungsfähige Netzeinspeisung**.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub settleable_feed_in: Decimal,
    /// `(17)` storage losses — A1 from `Z2`, A4 from `Z3`; not visible in A2/A3.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str_option"))]
    pub storage_losses: Option<Decimal>,
    /// `(18)` the settleable share of the store's output; undefined when `(6)` is zero.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str_option"))]
    pub settleable_share: Option<Decimal>,
    /// `(19)` the privilegeable part of the storage losses.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub privilegeable_losses: Decimal,
    /// `(20)` the quantity that reduces the levies.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub levy_reducing: Decimal,
    /// `(21)` **umlagebelasteter Netzbezug** — what levies are still owed on.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub levied_grid_draw: Decimal,
    /// `(26)` supportable feed-in straight from the plant.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub supported_direct: Decimal,
    /// `(28)` supportable feed-in out of the store, before the AW>0 share.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub supportable_storage_feed_in: Decimal,
    /// `(30)` the AW>0 share of the store's feed-in; undefined when `(11)` is zero.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str_option"))]
    pub supported_share_of_storage: Option<Decimal>,
    /// `(31)` supportable feed-in out of the store.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub supported_storage: Decimal,
    /// `(32)` **förderfähige Netzeinspeisung**.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub supported_feed_in: Decimal,
}

impl Abgrenzung {
    /// How much of the period's grid draw escaped the levies, as a fraction —
    /// the one number a household wants out of all of this.
    #[must_use]
    pub fn levy_relief_share(&self) -> Decimal {
        if self.grid_draw.is_zero() {
            Decimal::ZERO
        } else {
            self.levy_reducing / self.grid_draw
        }
    }
}

/// Evaluate the Abgrenzungsoption over one calendar month or Rumpfmonat,
/// `[MiSpeL A1 4.2]`, with `metering`'s formulas (1)–(32).
///
/// `quarters` must lie in one calendar month; `metering` refuses one that does
/// not ([`MispelError::OutsidePeriod`]).
///
/// # Errors
/// [`MispelError`] when there are no values, a value is negative, case A4 is
/// asked for without the separate storage meter, or the quarters span months.
#[allow(
    clippy::similar_names,
    reason = "z1nb, z1ne, z2v, z2e are the Festlegung's own meter names"
)]
pub fn abgrenzung_month(
    fall: Basisfall,
    rules: RuleSet,
    quarters: &[QuarterHour],
) -> Result<Abgrenzung, MispelError> {
    validate(
        quarters,
        &[
            ("Z1NB", |q| q.grid_draw),
            ("Z1NE", |q| q.grid_feed_in),
            ("Z2V", |q| q.device_consumption),
            ("Z2E", |q| q.device_generation),
        ],
    )?;
    if fall.needs_z3()
        && quarters
            .iter()
            .any(|q| q.storage_consumption.is_none() || q.storage_generation.is_none())
    {
        return Err(MispelError::MissingStorageMeter);
    }

    let z1nb = series(quarters, |q| q.grid_draw)?;
    let z1ne = series(quarters, |q| q.grid_feed_in)?;
    let z2v = series(quarters, |q| q.device_consumption)?;
    let z2e = series(quarters, |q| q.device_generation)?;
    let (z3v, z3e) = if fall.needs_z3() {
        (
            Some(series(quarters, |q| {
                q.storage_consumption.unwrap_or_default()
            })?),
            Some(series(quarters, |q| {
                q.storage_generation.unwrap_or_default()
            })?),
        )
    } else {
        (None, None)
    };
    let basisfall = match (fall, &z3v, &z3e) {
        (Basisfall::A1, ..) => ma::Basisfall::Stromspeicher,
        (Basisfall::A2, ..) => ma::Basisfall::Ladepunkt,
        (Basisfall::A3, ..) => ma::Basisfall::StromspeicherUndLadepunkt,
        (Basisfall::A4, Some(z3v), Some(z3e)) => ma::Basisfall::MitSpeicherzaehler { z3v, z3e },
        (Basisfall::A4, ..) => return Err(MispelError::MissingStorageMeter),
    };
    let supported = supported_quarters(quarters)?;
    let monat = ma::Abgrenzungsoption {
        z1nb: &z1nb,
        einspeisung: Einspeisung::Z1(&z1ne),
        z2v: &z2v,
        z2e: &z2e,
        basisfall,
        anlagen: ma::Anlagen::Eine(Gate::Flagged(&supported)),
    }
    .monat()?;
    let plant = monat
        .anlagen
        .first()
        .ok_or_else(|| MispelError::Metering("no plant in the month's result".into()))?;

    Ok(Abgrenzung {
        fall,
        rules,
        grid_draw: monat.f3,
        grid_feed_in: monat.f4,
        device_consumption: monat.f5,
        device_generation: monat.f6,
        grid_charged: monat.f9,
        plant_charged: monat.f10,
        device_feed_in_base: monat.f11,
        foreign_charge: monat.f12,
        device_feed_in_considered: monat.f13,
        efficiency: monat.f14,
        renewable_storage_output: monat.f15,
        settleable_feed_in: monat.f16,
        storage_losses: monat.f17,
        settleable_share: monat.f18,
        privilegeable_losses: monat.f19,
        levy_reducing: monat.f20,
        levied_grid_draw: monat.f21,
        supported_direct: plant.f26,
        supportable_storage_feed_in: monat.f28,
        supported_share_of_storage: plant.f30,
        supported_storage: plant.f31,
        supported_feed_in: monat.foerderfaehig()?,
    })
}

// ── Anlage 2 — the Pauschaloption ───────────────────────────────────────────

/// Which Basisfall of Anlage 2 the installation matches, `[MiSpeL A2 4.1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum PauschalFall {
    /// P1 — a storage system, no charge point.
    P1,
    /// P2 — a bidirectional charge point, no storage system.
    P2,
    /// P3 — both.
    P3,
}

/// The installation the Pauschaloption is applied to.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PauschalPlant {
    /// Which case.
    pub fall: PauschalFall,
    /// `Pinst` — installed solar power behind the Einspeisestelle, kWp.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub solar_kwp: Decimal,
    /// `SKinst` — installed storage capacity behind it, kWh. Ignored in P2.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub storage_kwh: Decimal,
}

impl PauschalPlant {
    fn basis(&self) -> Result<mp::Pauschalbasis, MispelError> {
        let capacity = || {
            if self.storage_kwh > Decimal::ZERO {
                Ok(self.storage_kwh)
            } else {
                Err(MispelError::NoStorageCapacity(self.fall))
            }
        };
        Ok(match self.fall {
            PauschalFall::P1 => mp::Pauschalbasis::Stromspeicher {
                kapazitaet_kwh: capacity()?,
            },
            PauschalFall::P2 => mp::Pauschalbasis::Ladepunkt,
            PauschalFall::P3 => mp::Pauschalbasis::StromspeicherUndLadepunkt {
                kapazitaet_kwh: capacity()?,
            },
        })
    }
}

/// The period one Pauschaloption evaluation covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum PauschalPeriod {
    /// A whole Berlin calendar year.
    Year(i32),
    /// A Rumpfjahr, inclusive at both ends, in one calendar year (Anlage 2 § 9).
    Rumpfjahr {
        /// First day.
        #[cfg_attr(feature = "serde", serde(with = "ems_core::wire::iso_date"))]
        first: Date,
        /// Last day.
        #[cfg_attr(feature = "serde", serde(with = "ems_core::wire::iso_date"))]
        last: Date,
    },
}

/// The days a Rumpfjahr's limits are prorated by (Anlage 2 § 9.1–9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RumpfjahrDays {
    /// First day.
    #[cfg_attr(feature = "serde", serde(with = "ems_core::wire::iso_date"))]
    pub first: Date,
    /// Last day.
    #[cfg_attr(feature = "serde", serde(with = "ems_core::wire::iso_date"))]
    pub last: Date,
    /// `(P22)R` — days of the Rumpfjahr.
    pub days: u16,
    /// `(P19)R` — of them, days April to September.
    pub summer_days: u16,
}

/// The Pauschaloption for one calendar year or Rumpfjahr, `[MiSpeL A2 4.2]`,
/// with the adopted numbering.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Pauschal {
    /// The installation.
    pub plant: PauschalPlant,
    /// Which version of the Festlegung.
    pub rules: RuleSet,
    /// The Rumpfjahr, where the period is one; its limits are the `R` ones.
    pub rumpfjahr: Option<RumpfjahrDays>,
    /// `(P1)` / `(P1)R` — Pauschalgrenze der Förderfähigkeit, kWh.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub support_cap: Decimal,
    /// `(P3)` / `(P3)R` — the Indifferenzbereich, kWh.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub indifference_band: Decimal,
    /// `(P4)` / `(P4)R` — Pauschalgrenze der Saldierungsfähigkeit, kWh.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub settlement_threshold: Decimal,
    /// `(P7)` — feed-in in quarter hours with a non-negative spot price.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub feed_in_at_non_negative_prices: Decimal,
    /// `(P8)` — of it, the part above the settlement threshold.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub above_threshold: Decimal,
    /// `(P9)` — total grid draw.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub grid_draw: Decimal,
    /// `(P10)` — **saldierungsfähige Netzeinspeisung**.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub settleable_feed_in: Decimal,
    /// `(P11)` — **umlagebelasteter Netzbezug**.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub levied_grid_draw: Decimal,
    /// `(P14)` — feed-in in quarter hours with a positive anzulegender Wert.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub feed_in_when_supported: Decimal,
    /// `(P16)` — **förderfähige Netzeinspeisung**.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub supported_feed_in: Decimal,
}

/// Evaluate the Pauschaloption over one calendar year or Rumpfjahr,
/// `[MiSpeL A2 4.2]`, with `metering`'s formulas (P1)–(P22).
///
/// Where the Abgrenzungsoption follows the electrons quarter by quarter, this
/// draws two flat lines through the year's feed-in — one at 500 kWh/kWp, one an
/// indifference band above it — and asks only which side of them a
/// kilowatt-hour fell on. It needs one meter rather than two.
///
/// # Errors
/// [`MispelError`] when there are no values, a value is negative, the
/// installation is over 30 kWp, a storage case has no capacity, or a quarter
/// hour lies outside the period.
#[allow(
    clippy::similar_names,
    reason = "z1nb, z1ne, z2v, z2e are the Festlegung's own meter names"
)]
pub fn pauschal_period(
    plant: PauschalPlant,
    rules: RuleSet,
    period: PauschalPeriod,
    quarters: &[QuarterHour],
) -> Result<Pauschal, MispelError> {
    validate(
        quarters,
        &[("Z1NB", |q| q.grid_draw), ("Z1NE", |q| q.grid_feed_in)],
    )?;
    if plant.solar_kwp > PAUSCHAL_MAX_KWP {
        return Err(MispelError::TooLargeForPauschal(plant.solar_kwp));
    }
    let z1nb = series(quarters, |q| q.grid_draw)?;
    let z1ne = series(quarters, |q| q.grid_feed_in)?;
    if let Some(missing) = quarters
        .iter()
        .find(|q| q.grid_feed_in > Decimal::ZERO && q.spot_price.is_none())
    {
        return Err(MispelError::MissingSpotPrice(missing.slot));
    }
    let sp = gate(quarters, |q| q.spot_non_negative() == Some(true));
    let aw = supported_quarters(quarters)?;
    let zeitraum = match period {
        PauschalPeriod::Year(year) => mp::Zeitraum::Kalenderjahr(year),
        PauschalPeriod::Rumpfjahr { first, last } => mp::Zeitraum::Rumpfjahr {
            erster_tag: first,
            letzter_tag: last,
        },
    };
    let jahr = mp::Pauschaloption {
        z1nb: &z1nb,
        einspeisung: Einspeisung::Z1(&z1ne),
        sp: Gate::Flagged(&sp),
        basis: plant.basis()?,
        anlagen: vec![mp::SolarAnlage {
            kwp: plant.solar_kwp,
            aw: Gate::Flagged(&aw),
        }],
        zeitraum,
    }
    .jahr()?;
    // The limits that apply — the Rumpfjahr's in a Rumpfjahr — as `metering`
    // states them; `kalenderjahr` holds the whole year's.
    let (support_cap, indifference_band, settlement_threshold) = (jahr.p1, jahr.p3, jahr.p4);
    let rumpfjahr = jahr.rumpfjahr.as_ref().map(|r| RumpfjahrDays {
        first: r.erster_tag,
        last: r.letzter_tag,
        days: r.tr,
        summer_days: r.trs,
    });
    let feed_in_when_supported = jahr.anlagen.iter().map(|a| a.p14).sum();

    Ok(Pauschal {
        plant,
        rules,
        rumpfjahr,
        support_cap,
        indifference_band,
        settlement_threshold,
        feed_in_at_non_negative_prices: jahr.p7,
        above_threshold: jahr.p8,
        grid_draw: jahr.p9,
        settleable_feed_in: jahr.p10,
        levied_grid_draw: jahr.p11,
        feed_in_when_supported,
        supported_feed_in: jahr.foerderfaehig()?,
    })
}

#[cfg(test)]
mod tests;
