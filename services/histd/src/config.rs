//! What `histd` is told.

/// Everything `histd` is configured with.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Settings {
    /// The shared daemon settings — `[service]` in the file.
    ///
    /// A **table** rather than a flattened set of top-level keys, so every
    /// daemon in this workspace is configured the same way (D160): an operator
    /// who has written `[service] listen = …` for one has written it for all of
    /// them. The environment override is unaffected either way — it goes through
    /// `AsMut<Settings>` rather than through the file's shape.
    #[serde(default)]
    pub service: ems_service::Settings,
    /// One bearer token per site — the credential its box presents.
    ///
    /// A box may read and write **its own** record and no other. Each is a
    /// [`ems_service::Secret`], so the usual deployment writes
    /// `haus-1 = "env:HEMS_HISTD_TOKEN_HAUS1"` and no credential enters the
    /// configuration file.
    ///
    /// Empty means this service accepts **nothing**, which is the safe reading
    /// of "nobody configured it": what these routes serve is a household's whole
    /// consumption record and the evidence a network operator settles on.
    #[serde(default)]
    pub site_tokens: std::collections::BTreeMap<String, ems_service::Secret>,
    /// Tokens that may read **any** site's § 14a evidence.
    ///
    /// A network operator checking a reduction it commanded, or an internal
    /// service building a portfolio view. They may not write — an operator that
    /// could write the record of its own control actions is marking its own
    /// homework — and they may not read the Data Act export, which is the
    /// household's under Article 4 and not theirs.
    /// Which households each tenant covers.
    ///
    /// A shared deployment hosting several operators names each one's
    /// households here, and an operator credential names the tenant it belongs
    /// to. A single-tenant deployment leaves this empty and writes
    /// `tenant = "*"` on its credential, which is the same reach stated out
    /// loud rather than arrived at by omission (D112).
    #[serde(default)]
    pub tenants: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    /// The operator credentials, each scoped to a tenant.
    #[serde(default)]
    pub operators: Vec<ems_service::OperatorCredential>,
    /// How to reach PostgreSQL.
    ///
    /// A fleet service, so a database a fleet can reach: what this daemon holds
    /// is every household's § 14a evidence, and a file on one node cannot be
    /// replicated, cannot be read by a second replica and cannot outlive the
    /// node (D156). The URL itself is a reference to a credential rather than
    /// the credential — see [`ems_service::DbSettings`].
    #[serde(default)]
    pub database: ems_service::DbSettings,
    /// How often to delete what has aged out, seconds.
    ///
    /// Daily. `[A1 7.3]`'s two years are not a number anybody is racing, and a
    /// gateway box has better things to do than sweep a table every minute.
    pub retention_sweep_s: u64,
    /// Which MiSpeL option each site settles under, `[MiSpeL Tenor]`.
    ///
    /// A **declared** fact about the installation, exactly like
    /// `Para9Status`: no meter reading says whether a storage system has ever
    /// been charged from the grid, whether a charge point is bidirectional, or
    /// which of the three options the Anlagenbetreiber chose. The Festlegung
    /// has them choose; this is where the choice is written down, so a Nachweis
    /// is computed under the option the household is actually on rather than
    /// under whichever one the arithmetic happens to support.
    ///
    /// A site that is absent from this map has not declared one, and
    /// `/v1/sites/{site}/mispel` answers `404` rather than guessing — settling
    /// a household under the wrong Basisfall produces a Nachweis that is
    /// arithmetically perfect and about a different installation.
    #[serde(default)]
    pub mispel: std::collections::BTreeMap<String, MispelSettings>,
    /// The day the European Commission approved § 19 Abs. 3c EEG under state-aid
    /// law, `[MiSpeL Tenor 9]` b) — `"YYYY-MM-DD"`, or absent while there is no
    /// approval. The Pauschaloption applies from the first day of the month
    /// after it, and not at all before it.
    #[serde(default, with = "ems_core::wire::iso_date::option")]
    pub mispel_state_aid_approved_on: Option<time::Date>,
    /// The Model Context Protocol surface, off by default.
    ///
    /// It holds a household's data, so a token is **required** when it is
    /// switched on and the surface answers as whatever authority that token
    /// already carries here — an operator, or one site.
    #[serde(default)]
    pub mcp: ems_service::McpSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            service: ems_service::Settings::default(),
            site_tokens: std::collections::BTreeMap::new(),
            tenants: std::collections::BTreeMap::new(),
            operators: Vec::new(),
            mispel: std::collections::BTreeMap::new(),
            mispel_state_aid_approved_on: None,
            database: ems_service::DbSettings::default(),
            retention_sweep_s: 24 * 3600,
            mcp: ems_service::McpSettings::default(),
        }
    }
}

impl AsMut<ems_service::Settings> for Settings {
    fn as_mut(&mut self) -> &mut ems_service::Settings {
        &mut self.service
    }
}

/// Which MiSpeL option one site settles under, and the facts its arithmetic
/// needs.
///
/// The three options of the Festlegung, and the two that need arithmetic carry
/// the case they are evaluated under — a fact about what is behind the
/// Einspeisestelle rather than about the meter readings, which is why it is
/// declared and not derived.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, tag = "option", rename_all = "kebab-case")]
pub enum MispelSettings {
    /// **Ausschließlichkeit** — the store is never charged from the grid, so no
    /// energy through it is ever grey and there is nothing to separate.
    ///
    /// It settles nothing and is nevertheless **checked**. The claim is a
    /// statement about the registers, and `(1)¼ = MIN[Z1NB¼ ; Z2V¼]` — the
    /// gleichzeitiger Netzbezug — is what measures it: the option holds exactly
    /// while that is zero in every quarter hour of the period. The export
    /// computes it and names the quarter hours where it is not, because the
    /// alternative is a household learning that its levy privilege lapsed from
    /// its network operator rather than from its own box. `[MiSpeL A1 2.1.5]`
    /// for the formula, and D142 for the planner's half — it refuses to
    /// *schedule* a simultaneous charge, and this is what says none happened.
    Ausschliesslichkeit,
    /// **Abgrenzung** (Anlage 1) — the quarter-hourly formulas (1)–(32),
    /// settled per calendar month or Rumpfmonat.
    Abgrenzung {
        /// Which of the four Basisfälle the installation is, `[MiSpeL A1 3]`.
        basisfall: ems_grid::mispel::Basisfall,
        /// The first day the installation is assigned on the option's basis,
        /// `"YYYY-MM-DD"` — the first of a month unless `first_assignment`
        /// (§ 21b Abs. 1 S. 2 EEG by analogy); `histd` refuses to start otherwise.
        #[serde(with = "ems_core::wire::iso_date")]
        assigned_from: time::Date,
        /// Whether this is the **first** assignment of a newly commissioned
        /// plant, which alone may start mid-month (a Rumpfmonat or Rumpfjahr).
        #[serde(default)]
        first_assignment: bool,
        /// Whether the network and metering operators consented — required
        /// until 30.09.2027, `[MiSpeL Tenor 9]` a).
        #[serde(default)]
        operator_consent: bool,
    },
    /// **Pauschal** (Anlage 2) — the annual flat lines (P1)–(P16), for solar of
    /// at most 30 kWp, settled per calendar year or Rumpfjahr.
    Pauschal {
        /// Which of the three Pauschalfälle, `[MiSpeL A2 3]`.
        fall: ems_grid::mispel::PauschalFall,
        /// `Pinst` — installed solar power behind the Einspeisestelle, kWp.
        solar_kwp: f64,
        /// `SKinst` — installed storage capacity behind it, kWh.
        ///
        /// It matters more than its size suggests: `(P2)` widens the
        /// indifference band as the store shrinks against the roof, because a
        /// small store leaves less room for the arbitrage the band exists to
        /// keep out.
        storage_kwh: f64,
        /// The first day the installation is assigned on the option's basis,
        /// `"YYYY-MM-DD"` — the first of a month unless `first_assignment`
        /// (§ 21b Abs. 1 S. 2 EEG by analogy); `histd` refuses to start otherwise.
        #[serde(with = "ems_core::wire::iso_date")]
        assigned_from: time::Date,
        /// Whether this is the **first** assignment of a newly commissioned
        /// plant, which alone may start mid-month (a Rumpfmonat or Rumpfjahr).
        #[serde(default)]
        first_assignment: bool,
        /// Whether the network and metering operators consented — required
        /// until 30.09.2027, `[MiSpeL Tenor 9]` a).
        #[serde(default)]
        operator_consent: bool,
    },
}

/// The declared facts the governed window depends on, `[MiSpeL Tenor 5]` and
/// `[MiSpeL Tenor 9]` a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssignmentSettings {
    /// The first day the installation is assigned on the option's basis.
    pub assigned_from: time::Date,
    /// Whether this is the first assignment of a newly commissioned plant.
    pub first_assignment: bool,
    /// Whether the network and metering operators consented.
    pub operator_consent: bool,
}

impl MispelSettings {
    /// The assignment facts of an option that settles anything.
    #[must_use]
    pub const fn assignment(&self) -> Option<AssignmentSettings> {
        match self {
            MispelSettings::Ausschliesslichkeit => None,
            MispelSettings::Abgrenzung {
                assigned_from,
                first_assignment,
                operator_consent,
                ..
            }
            | MispelSettings::Pauschal {
                assigned_from,
                first_assignment,
                operator_consent,
                ..
            } => Some(AssignmentSettings {
                assigned_from: *assigned_from,
                first_assignment: *first_assignment,
                operator_consent: *operator_consent,
            }),
        }
    }
}

impl Settings {
    /// Refuse a MiSpeL declaration the Festlegung does not permit.
    ///
    /// Checked at start-up rather than at the first export, because a
    /// mid-month switch of an existing plant is a Rumpfmonat that cannot
    /// exist, and an operator should learn that before anybody asks for the
    /// Nachweis that would rest on it.
    ///
    /// # Errors
    /// The site and the [`ems_grid::mispel::Refusal`] its assignment draws.
    pub fn validate_mispel(&self) -> Result<(), String> {
        let rules = ems_grid::mispel::RuleSet::default();
        for (site, declared) in &self.mispel {
            if let Some(a) = declared.assignment() {
                ems_grid::mispel::Assignment {
                    from: a.assigned_from,
                    first: a.first_assignment,
                }
                .validate(rules)
                .map_err(|e| format!("[mispel.{site}]: {e}"))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example file that ships with the daemon.
    ///
    /// Parsed by a test rather than trusted, because a commented example that
    /// has drifted from the struct it documents is worse than none: it is read
    /// by whoever is deploying this, and every line of it looks authoritative.
    /// `include_str!` makes it a build input.
    const EXAMPLE: &str = include_str!("../histd.example.toml");

    #[test]
    fn the_example_configuration_parses_and_describes_a_service() {
        let settings: Settings = toml::from_str(EXAMPLE).expect("the shipped example parses");
        assert!(
            !settings.site_tokens.is_empty(),
            "an example with no site token would document a service that refuses \
             every box"
        );
        assert!(
            !settings.operators.is_empty(),
            "…and one with no operator would document a Nachweis nobody can read"
        );
        assert!(
            settings.database.tls,
            "what crosses this socket is a fleet's § 14a evidence"
        );
        assert!(
            settings.database.statement_timeout_s > 0,
            "a serving replica needs a bound on a query whose reader has gone"
        );
    }

    /// Evidence for SR-028.
    #[test]
    fn a_mid_month_switch_of_an_existing_plant_stops_the_daemon() {
        let toml = r#"
            [mispel.haus-1]
            option = "abgrenzung"
            basisfall = "a1"
            assigned_from = "2026-11-12"
        "#;
        let settings: Settings = toml::from_str(toml).expect("it parses");
        let error = settings
            .validate_mispel()
            .expect_err("§ 21b Abs. 1 S. 2 EEG");
        assert!(error.contains("haus-1") && error.contains("21b"), "{error}");

        let first = toml.replace(
            "assigned_from = \"2026-11-12\"",
            "assigned_from = \"2026-11-12\"\nfirst_assignment = true",
        );
        let settings: Settings = toml::from_str(&first).expect("it parses");
        assert_eq!(
            settings.validate_mispel(),
            Ok(()),
            "a new plant may start mid-month"
        );
    }

    #[test]
    fn a_date_that_is_not_one_is_refused_when_the_file_is_read() {
        let toml = r#"
            [mispel.haus-1]
            option = "abgrenzung"
            basisfall = "a1"
            assigned_from = "2026-13-01"
        "#;
        assert!(toml::from_str::<Settings>(toml).is_err());
    }

    #[test]
    fn the_example_declares_a_mispel_option_that_the_export_can_settle() {
        // A site that has declared none is refused rather than settled under a
        // guess, so an example that declared none would document the one case
        // that produces no document at all.
        let settings: Settings = toml::from_str(EXAMPLE).expect("the shipped example parses");
        assert!(settings.mispel.values().next().is_some());
        assert_eq!(
            settings.validate_mispel(),
            Ok(()),
            "and the Festlegung permits it"
        );
    }
}
