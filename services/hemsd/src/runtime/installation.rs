//! Activate reviewed household configuration changes and recover interrupted installations.
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use aws_lc_rs::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::handover::InverterReview;

/// Lifecycle of a single reviewed installation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// The candidate is on disk or being started.
    Restarting,
    /// Every selected device supplied its expected measurement.
    Applied,
    /// The previous file is back in force.
    Restored,
    /// A manual edit superseded the transaction.
    Superseded,
}

/// Supported installation roles. Each has a specific model check and runtime readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceRole {
    /// Roof inverter with active-power limiting.
    Pv,
    /// Three-phase meter at the site boundary.
    ConnectionMeter,
    /// Read-only inverter in shadow mode.
    ObservedPv,
    /// Read-only battery in shadow mode.
    ObservedBattery,
}

impl DeviceRole {
    /// Asset identifier in the household site.
    #[must_use]
    pub fn asset(self) -> &'static str {
        match self {
            Self::Pv | Self::ObservedPv => "pv",
            Self::ObservedBattery => "battery",
            Self::ConnectionMeter => "netzanschluss-zaehler",
        }
    }
}

/// The most recent change survives a restart and an interrupted browser request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Receipt {
    /// Equipment selected by the household.
    pub role: DeviceRole,
    /// Opaque id issued with the reviewed change.
    pub id: String,
    /// Result visible in Setup after the gateway returns.
    pub outcome: Outcome,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum MeterEndpoint {
    Tcp { address: std::net::SocketAddr },
    Serial { bus: crate::config::SerialBus },
}

impl MeterEndpoint {
    pub(super) async fn inspect(&self, unit: u8) -> Option<ems_drv::modbus::probe::Identity> {
        match self {
            Self::Tcp { address } => super::handover::inspect(*address, unit).await,
            Self::Serial { bus } => super::serial::inspect(bus, unit).await,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Device {
    pub(super) role: DeviceRole,
    #[serde(flatten)]
    endpoint: MeterEndpoint,
    unit: u8,
    manufacturer: String,
    model: String,
    serial: String,
}

impl Device {
    pub(super) fn from_identity(
        role: DeviceRole,
        endpoint: MeterEndpoint,
        unit: u8,
        identity: &ems_drv::modbus::probe::Identity,
    ) -> Self {
        Self {
            role,
            endpoint,
            unit,
            manufacturer: identity.manufacturer.clone(),
            model: identity.model.clone(),
            serial: identity.serial.clone(),
        }
    }
}

impl DeviceRole {
    /// Whether this reading contains the quantity promised in Setup.
    #[must_use]
    pub fn measured(self, measurement: &ems_core::prelude::Measurement) -> bool {
        measurement.quality == metering::QualityFlag::Measured
            && match self {
                Self::ObservedBattery => measurement.soc.is_some(),
                Self::ObservedPv | Self::Pv | Self::ConnectionMeter => measurement.power.is_some(),
            }
    }

    pub(super) fn supports(self, id: &ems_drv::modbus::probe::Identity) -> bool {
        match self {
            Self::Pv => id.supports_pv_driver(),
            Self::ConnectionMeter => id.connection_meter,
            Self::ObservedPv => id.inverter_model.is_some(),
            Self::ObservedBattery => id.storage_meter,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Journal {
    devices: Vec<Device>,
    receipt: Receipt,
    before: String,
    after: String,
}

struct Proposal {
    devices: Vec<Device>,
    id: String,
    block: String,
}

struct Proposals {
    source: String,
    entries: VecDeque<Proposal>,
}

/// One writer across the process and all browser sessions.
pub struct Installer {
    path: PathBuf,
    proposals: Mutex<Proposals>,
    _exclusive: File,
    restarting: AtomicBool,
}

/// Owned messages keep configuration contents out of HTTP errors.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    /// An unknown proposal or a different source configuration.
    #[error("The configuration changed. Discover the device and review it again.")]
    Stale,
    /// The transaction could not be made durable.
    #[error("The configuration file could not be updated. Check its directory permissions.")]
    Storage(#[from] io::Error),
}

impl Installer {
    /// Lock the configuration and recover an interrupted change before loading it.
    ///
    /// # Errors
    /// Returns a storage error if a pending transaction cannot be recovered.
    pub fn open(path: &Path) -> io::Result<Self> {
        let path = path.canonicalize()?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_added_extension("ems-lock"))?;
        lock.try_lock().map_err(io::Error::from)?;
        let installer = Self {
            path,
            proposals: Mutex::new(Proposals {
                source: String::new(),
                entries: VecDeque::new(),
            }),
            _exclusive: lock,
            restarting: AtomicBool::new(false),
        };
        installer.rollback()?;
        installer.begin()?;
        Ok(installer)
    }

    /// Bind subsequent reviews to the file read on this gateway start.
    ///
    /// # Errors
    /// Returns a storage error if the file cannot be read.
    pub fn begin(&self) -> io::Result<()> {
        let mut proposals = self
            .proposals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        proposals.source = hash(&fs::read(&self.path)?);
        proposals.entries.clear();
        Ok(())
    }

    /// Register one validated server-built addition.
    pub fn offer(
        &self,
        review: &mut InverterReview,
        identity: &ems_drv::modbus::probe::Identity,
    ) -> Option<String> {
        if review.refusal.is_some() || !identity.supports_pv_driver() {
            return None;
        }
        let original = fs::read_to_string(&self.path).ok()?;
        let candidate = format!("{original}\n{}", review.toml);
        if toml::from_str::<crate::Settings>(&candidate).is_err() {
            review.refusal =
                Some("The proposed addition conflicts with the current TOML structure.".into());
            return None;
        }
        Some(self.register(
            &review.toml,
            vec![Device::from_identity(
                DeviceRole::Pv,
                MeterEndpoint::Tcp {
                    address: review.address,
                },
                review.unit,
                identity,
            )],
        ))
    }

    /// Register the verified connection meter and optional observed assets on an empty site.
    pub(super) fn offer_site(
        &self,
        block: &str,
        devices: Vec<Device>,
    ) -> io::Result<Option<String>> {
        Ok((self.empty_site()?
            && devices
                .first()
                .is_some_and(|d| d.role == DeviceRole::ConnectionMeter))
        .then(|| self.register(block, devices)))
    }

    /// A missing installation has no site-specific TOML yet.
    ///
    /// # Errors
    /// Returns a read error for an inaccessible configuration file.
    pub fn empty_site(&self) -> io::Result<bool> {
        let source = fs::read_to_string(&self.path)?;
        let table: toml::Table = toml::from_str(&source).map_err(io::Error::other)?;
        Ok(table
            .keys()
            .all(|key| matches!(key.as_str(), "serial_buses" | "heat_meter")))
    }

    fn register(&self, block: &str, devices: Vec<Device>) -> String {
        let mut proposals = self
            .proposals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if proposals.entries.len() == super::handover::MAX_HOSTS {
            proposals.entries.pop_front();
        }
        let id = Uuid::new_v4().to_string();
        proposals.entries.push_back(Proposal {
            devices,
            id: id.clone(),
            block: block.to_owned(),
        });
        id
    }

    /// Append exactly the reviewed block, preserving all other bytes.
    ///
    /// # Errors
    /// Refuses a stale, edited or unissued proposal; reports a storage failure without secrets.
    pub fn apply(&self, id: &str) -> Result<(Receipt, bool), ApplyError> {
        let proposals = self
            .proposals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = self.journal()? {
            if previous.receipt.id == id {
                return Ok((previous.receipt, false));
            }
            if previous.receipt.outcome == Outcome::Restarting {
                return Err(ApplyError::Stale);
            }
        }
        let proposal = proposals
            .entries
            .iter()
            .find(|p| p.id == id)
            .ok_or(ApplyError::Stale)?;
        let original = fs::read(&self.path)?;
        if hash(&original) != proposals.source {
            return Err(ApplyError::Stale);
        }
        // The initial proposal includes all provisioned bus tables. Appending its
        // root fields after a TOML table would change their scope.
        let mut candidate = if proposal
            .devices
            .first()
            .is_some_and(|d| d.role == DeviceRole::ConnectionMeter)
        {
            Vec::new()
        } else {
            original.clone()
        };
        candidate.extend_from_slice(b"\n");
        candidate.extend_from_slice(proposal.block.as_bytes());
        let transaction = Journal {
            devices: proposal.devices.clone(),
            receipt: Receipt {
                role: proposal.devices[0].role,
                id: id.to_owned(),
                outcome: Outcome::Restarting,
            },
            before: proposals.source.clone(),
            after: hash(&candidate),
        };
        replace(&self.path.with_added_extension("ems-previous"), &original)?;
        self.record(&transaction)?;
        if let Err(error) = replace(&self.path, &candidate) {
            self.rollback()?;
            return Err(error.into());
        }
        self.request_restart();
        Ok((transaction.receipt, true))
    }

    /// Schedule a fresh gateway runtime after draining requests.
    pub fn request_restart(&self) {
        self.restarting.store(true, Ordering::Release);
    }

    /// Consume a restart request in the supervisor.
    pub fn take_restart(&self) -> bool {
        self.restarting.swap(false, Ordering::AcqRel)
    }

    /// Last recorded outcome.
    ///
    /// # Errors
    /// Returns a storage error if the journal cannot be decoded.
    pub fn receipt(&self) -> io::Result<Option<Receipt>> {
        Ok(self.journal()?.map(|j| j.receipt))
    }

    /// Check the recorded endpoint and reported identity again before enabling a new driver.
    ///
    /// # Errors
    /// Returns a storage error if the transaction cannot be read.
    pub async fn candidate_ready(&self) -> io::Result<bool> {
        let Some(journal) = self.pending()? else {
            return Ok(true);
        };
        if hash(&fs::read(&self.path)?) != journal.after {
            return Ok(false);
        }
        for device in journal.devices {
            let matches = device
                .endpoint
                .inspect(device.unit)
                .await
                .is_some_and(|id| {
                    device.role.supports(&id)
                        && device.manufacturer == id.manufacturer
                        && device.model == id.model
                        && device.serial == id.serial
                });
            if !matches {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Which device needs to provide a new measurement on this startup.
    ///
    /// # Errors
    /// Returns an error for an unreadable journal.
    pub fn trial_assets(&self) -> io::Result<Option<Vec<DeviceRole>>> {
        Ok(self
            .pending()?
            .map(|j| j.devices.into_iter().map(|d| d.role).collect()))
    }

    /// Confirm the candidate after the selected devices report.
    ///
    /// # Errors
    /// Returns a storage error if the result cannot be made durable.
    pub fn confirm(&self) -> io::Result<()> {
        if let Some(mut j) = self.pending()? {
            j.receipt.outcome = if hash(&fs::read(&self.path)?) == j.after {
                Outcome::Applied
            } else {
                Outcome::Superseded
            };
            self.record(&j)?;
        }
        Ok(())
    }

    /// Recover the previous configuration unless an external editor superseded it.
    ///
    /// # Errors
    /// Refuses to restore an altered backup.
    pub fn rollback(&self) -> io::Result<()> {
        if let Some(mut j) = self.pending()? {
            let current = hash(&fs::read(&self.path)?);
            j.receipt.outcome = if current == j.after {
                let previous = fs::read(self.path.with_added_extension("ems-previous"))?;
                if hash(&previous) != j.before {
                    return Err(io::Error::other("previous configuration checksum mismatch"));
                }
                replace(&self.path, &previous)?;
                Outcome::Restored
            } else if current == j.before {
                Outcome::Restored
            } else {
                Outcome::Superseded
            };
            self.record(&j)?;
        }
        Ok(())
    }

    fn pending(&self) -> io::Result<Option<Journal>> {
        Ok(self
            .journal()?
            .filter(|j| j.receipt.outcome == Outcome::Restarting))
    }

    fn journal(&self) -> io::Result<Option<Journal>> {
        match fs::read(self.path.with_added_extension("apply.json")) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn record(&self, journal: &Journal) -> io::Result<()> {
        replace(
            &self.path.with_added_extension("apply.json"),
            &serde_json::to_vec(journal)?,
        )
    }
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(digest(&SHA256, bytes))
}

fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = path.with_added_extension(format!("{}.tmp", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        #[cfg(unix)]
        File::open(
            path.parent()
                .ok_or_else(|| io::Error::other("missing configuration directory"))?,
        )?
        .sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ems_drv::modbus::probe::Identity;

    const BEFORE: &[u8] = b"# fixture retained byte-for-byte\n";

    struct Site(PathBuf);
    impl Site {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!("ems-install-{}", Uuid::new_v4()));
            fs::create_dir(&directory).unwrap();
            fs::write(directory.join("house.toml"), BEFORE).unwrap();
            Self(directory)
        }
        fn path(&self) -> PathBuf {
            self.0.join("house.toml")
        }
        fn start(&self) -> Installer {
            Installer::open(&self.path()).unwrap()
        }
    }
    impl Drop for Site {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn offer(installer: &Installer) -> String {
        installer
            .offer(
                &mut InverterReview {
                    proposal: None,
                    toml: "[[drivers]]\nkind = \"sunspec\"\nasset = \"pv\"\naddress = \"127.0.0.1:1502\"\n".into(),
                    refusal: None,
                    unit: 1,
                    address: "127.0.0.1:1502".parse().unwrap(),
                },
                &Identity {
                    base: 40000,
                    inverter_model: Some(103),
                    has_immediate_controls: true,
                    connection_meter: false,
                    storage_meter: false,
                    manufacturer: "virtual".into(),
                    model: "PV".into(),
                    serial: "example".into(),
                    version: String::new(),
                },
            )
            .unwrap()
    }

    #[test]
    fn simultaneous_confirmations_install_only_one_original_proposal() {
        let site = Site::new();
        let installer = site.start();
        let first = offer(&installer);
        let second = offer(&installer);
        assert!(matches!(installer.apply("edited"), Err(ApplyError::Stale)));
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let handles = [&first, &second].map(|id| {
                scope.spawn(|| {
                    barrier.wait();
                    installer.apply(id)
                })
            });
            assert_eq!(
                handles
                    .into_iter()
                    .filter_map(|h| h.join().unwrap().ok())
                    .count(),
                1
            );
        });
        let receipt = installer.receipt().unwrap().unwrap();
        assert_eq!(installer.apply(&receipt.id).unwrap(), (receipt, false));
        assert!(Installer::open(&site.path()).is_err());
    }

    #[test]
    fn each_durable_boundary_recovers_without_overwriting_an_external_edit() {
        // SR-097: close the process after backup, journal, replace or confirmation.
        for cut in 0..5 {
            let site = Site::new();
            let installer = site.start();
            let id = offer(&installer);
            let receipt = installer.apply(&id).unwrap().0;
            let candidate_bytes = fs::read(site.path()).unwrap();
            match cut {
                0 => {
                    fs::write(site.path(), BEFORE).unwrap();
                    fs::remove_file(site.path().with_added_extension("apply.json")).unwrap();
                }
                1 => fs::write(site.path(), BEFORE).unwrap(),
                2 => {}
                3 => installer.confirm().unwrap(),
                _ => fs::write(site.path(), b"# hand edited").unwrap(),
            }
            drop(installer);
            let reopened = site.start();
            let observed = reopened.receipt().unwrap().map(|r| {
                assert_eq!(r.id, receipt.id);
                r.outcome
            });
            match cut {
                0 => {
                    assert!(observed.is_none());
                    assert_eq!(fs::read(site.path()).unwrap(), BEFORE);
                }
                1 | 2 => {
                    assert_eq!(observed, Some(Outcome::Restored));
                    assert_eq!(fs::read(site.path()).unwrap(), BEFORE);
                }
                3 => {
                    assert_eq!(observed, Some(Outcome::Applied));
                    assert_eq!(fs::read(site.path()).unwrap(), candidate_bytes);
                }
                _ => {
                    assert_eq!(observed, Some(Outcome::Superseded));
                    assert_eq!(fs::read(site.path()).unwrap(), b"# hand edited");
                }
            }
        }
    }
}
