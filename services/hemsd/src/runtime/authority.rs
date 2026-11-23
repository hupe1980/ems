//! Household authorization for the installer to transfer an observed site.

use aws_lc_rs::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};

use crate::Settings;
use crate::config::OperatingMode;
use crate::store::Store;

/// A single commissioning visit, retained across a gateway restart.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Transfer {
    /// A new review is issued when the site or independent controller changes.
    pub id: String,
    /// Controller responsible until the installer completes the transfer.
    pub controller: String,
    /// Latest time the installer can use the household's authorization, Unix seconds.
    pub approved_until: Option<i64>,
    scope: String,
}

/// Evidence visible from the observing gateway at review time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    /// Connection meter and every required actuator can be checked.
    Ready,
    /// The connection meter has no current measurement.
    MeterMissing,
    /// An actuator lacks a verified command path.
    ActuatorMissing,
}

/// The household's authorization and current commissioning evidence.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct Review {
    /// Commissioning visit and any existing authorization.
    #[serde(flatten)]
    pub transfer: Transfer,
    /// Whether the observer has verified the site's control surfaces.
    pub readiness: Readiness,
}

pub(super) fn readiness(
    site: &ems_core::prelude::Site,
    observed: &crate::drivers::Observed,
) -> Readiness {
    use ems_core::prelude::Asset;
    if observed
        .state
        .grid
        .is_none_or(|m| m.power.is_none() || m.quality != metering::QualityFlag::Measured)
    {
        return Readiness::MeterMissing;
    }
    // The available read-only probe can establish a model 123 PV command path.
    // A generic "accepts commands" flag also covers relays and cannot establish
    // signed battery power, EV current or compressor control.
    if site.assets.iter().any(|asset| match asset {
        Asset::Pv(_) => {
            !observed.curtailable.contains(asset.id())
                || observed
                    .state
                    .asset(asset.id())
                    .is_none_or(|m| m.power.is_none())
        }
        Asset::Meter(_) => false,
        Asset::Load(load) => !matches!(load.kind, ems_core::asset::LoadKind::Fixed),
        _ => true,
    }) {
        return Readiness::ActuatorMissing;
    }
    Readiness::Ready
}

/// Household decision on the current handover review.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    /// Review id shown to the household.
    pub id: String,
    /// Whether the household authorizes the installer to proceed.
    pub approve: bool,
}

pub(super) fn enter(
    settings: &Settings,
    store: Option<&Store>,
    now: i64,
    ready: bool,
) -> anyhow::Result<bool> {
    let Some(store) = store else {
        anyhow::ensure!(
            settings.mode != OperatingMode::Shadow,
            "shadow mode requires store_path to retain household authority"
        );
        return Ok(false);
    };
    let existing = store.transfer()?;
    let meters_and_actuators = settings
        .drivers
        .iter()
        .filter(|d| {
            matches!(
                d,
                crate::config::DriverSettings::Sunspec(_)
                    | crate::config::DriverSettings::Registers(_)
            )
        })
        .collect::<Vec<_>>();
    let scope = hex::encode(digest(
        &SHA256,
        &serde_json::to_vec(&(&settings.site, meters_and_actuators))?,
    ));
    if settings.mode == OperatingMode::Shadow {
        let controller = settings
            .external_limit_controller
            .as_deref()
            .unwrap_or_default()
            .trim();
        if existing.is_none_or(|old| old.controller != controller || old.scope != scope) {
            store.put_transfer(Some(&Transfer {
                id: uuid::Uuid::new_v4().to_string(),
                controller: controller.into(),
                approved_until: None,
                scope,
            }))?;
        }
    } else if let Some(old) = existing {
        anyhow::ensure!(
            old.scope == scope
                && old
                    .approved_until
                    .is_some_and(|end| end >= now && end <= now.saturating_add(3600)),
            "the household must authorize handover for this site in shadow Setup within the last hour"
        );
        anyhow::ensure!(
            ready,
            "handover requires a connection meter and command drivers for all controllable devices"
        );
        return Ok(true);
    }
    Ok(false)
}

impl Transfer {
    pub(super) fn review(&mut self, approval: &Approval, now: i64) -> bool {
        if self.id != approval.id {
            return false;
        }
        self.approved_until = approval.approve.then(|| now.saturating_add(3600));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn household_review_requires_measured_boundary_and_the_matching_actuator_model() {
        use crate::{Household, HouseholdConfig};
        use ems_core::{measurement::Measurement, prelude::Power};
        let house = Household::build(&HouseholdConfig {
            battery: None,
            evse: None,
            heat_pump: None,
            dhw: None,
            dishwasher: None,
            ..HouseholdConfig::default()
        })
        .unwrap();
        let mut observed = crate::drivers::Observed::default();
        assert_eq!(readiness(&house.site, &observed), Readiness::MeterMissing);
        let mut m = Measurement::at(time::OffsetDateTime::UNIX_EPOCH);
        m.power = Some(Power::ZERO);
        observed.state.grid = Some(m);
        assert_eq!(
            readiness(&house.site, &observed),
            Readiness::ActuatorMissing
        );
        let pv = house.pv.unwrap();
        observed.state.assets.insert(pv.clone(), m);
        assert_eq!(
            readiness(&house.site, &observed),
            Readiness::ActuatorMissing
        );
        observed.curtailable.insert(pv);
        assert_eq!(readiness(&house.site, &observed), Readiness::Ready);
        assert_eq!(
            readiness(
                &Household::build(&HouseholdConfig::default()).unwrap().site,
                &observed
            ),
            Readiness::ActuatorMissing
        );
    }

    #[test]
    fn handover_needs_matching_household_approval_live_withdrawal_and_complete_commissioning() {
        let store = Store::in_memory().unwrap();
        let mut shadow = Settings {
            mode: OperatingMode::Shadow,
            external_limit_controller: Some("Panel A".into()),
            ..Settings::default()
        };
        let mut managed = Settings::default();
        enter(&shadow, Some(&store), 0, true).unwrap();
        let mut pending = store.transfer().unwrap().unwrap();
        let mut approval = Approval {
            id: pending.id.clone(),
            approve: true,
        };
        assert!(enter(&managed, Some(&store), 1000, true).is_err());
        assert!(pending.review(&approval, 1000));
        store.put_transfer(Some(&pending)).unwrap();
        assert!(enter(&managed, Some(&store), 1000, true).unwrap());
        assert!(enter(&managed, Some(&store), 1000, false).is_err());
        assert!(enter(&managed, Some(&store), 4601, true).is_err());
        managed.site.pv_kwp += 1.;
        assert!(enter(&managed, Some(&store), 1000, true).is_err());
        approval.approve = false;
        assert!(pending.review(&approval, 1000));
        store.put_transfer(Some(&pending)).unwrap();
        assert!(enter(&Settings::default(), Some(&store), 1000, true).is_err());
        shadow.external_limit_controller = Some("Panel B".into());
        enter(&shadow, Some(&store), 1000, true).unwrap();
        assert!(!store.transfer().unwrap().unwrap().review(&approval, 1000));
    }
}
