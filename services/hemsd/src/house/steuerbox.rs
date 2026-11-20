//! The network operator's box: an EEBUS Energy Guard that dials the household
//! and writes the reference day's § 14a reduction at the reference day's time.
//!
//! The same `EnergyGuardActor` `eebus` ships in its own Steuerbox example; what
//! is added here is the schedule.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eebus::cert::{self, CertParams, Identity};
use eebus::model::{DeviceType, EntityType};
use eebus::runtime::{Hub, HubEvent, Node, TrustStore};
use eebus::spine::{Engine, LocalDevice, LocalEntity};
use eebus::tls::ShipTls;
use eebus::usecases::limitation::{self, EnergyGuardActor, GuardEvent, LimitWrite};
use eebus::usecases::lpc;
use time::Duration;
use tokio::sync::watch;

use super::Clock;

const SHIP_ID: &str = "n:hems-house_Steuerbox-1";

/// What the Steuerbox has seen of the household so far.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Seen {
    /// The SHIP session is up.
    pub connected: bool,
    /// The last limit the household acknowledged, watts.
    pub accepted_w: Option<f64>,
}

/// The reduction of the day: from, until (both since local midnight), watts.
pub type Reduction = (Duration, Duration, f64);

/// The Steuerbox's certificate and key, kept in `path` so its SKI — which the
/// box's configuration names — survives a restart of the house.
///
/// # Errors
/// Where the file cannot be read, parsed or written.
pub fn identity(path: Option<&std::path::Path>) -> anyhow::Result<Identity> {
    const SPLIT: &str = "-----BEGIN PRIVATE KEY-----";
    if let Some(path) = path
        && let Ok(kept) = std::fs::read_to_string(path)
        && let Some(at) = kept.find(SPLIT)
    {
        return Identity::from_pem(&kept[..at], &kept[at..])
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()));
    }
    let identity = cert::self_signed(CertParams::new(SHIP_ID))
        .map_err(|e| anyhow::anyhow!("the Steuerbox certificate: {e}"))?;
    if let Some(path) = path {
        // A private key, even a simulator's: whoever holds it can reduce any
        // box that was told to trust this SKI.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        std::io::Write::write_all(
            &mut options.open(path)?,
            format!("{}{}", identity.certificate_pem(), identity.key_pem()).as_bytes(),
        )?;
    }
    Ok(identity)
}

fn guard() -> (Engine, EnergyGuardActor) {
    let mut device = LocalDevice::new(
        "n:hems-house",
        "Steuerbox-1",
        DeviceType::ElectricitySupplySystem,
    )
    .expect("a valid device address");
    device
        .add_entity(
            LocalEntity::new([1], EntityType::GridGuard)
                .with_feature(limitation::client_feature(1))
                .with_feature(limitation::device_diagnosis_feature(2)),
        )
        .expect("a fresh entity");
    let client = device.address_of(&[1], 1);
    let diagnosis = device.address_of(&[1], 2);
    let mut engine = Engine::new(device);
    engine.add_use_case([1], 1, &lpc::ENERGY_GUARD);
    let actor = EnergyGuardActor::new(
        lpc::DIRECTION,
        client,
        diagnosis,
        core::time::Duration::ZERO,
    );
    (engine, actor)
}

/// What the grid requires at `since_midnight`.
fn required(reduction: Option<Reduction>, since_midnight: Duration) -> LimitWrite {
    match reduction {
        Some((from, until, watts)) if since_midnight >= from && since_midnight < until => {
            let left = (until - since_midnight).unsigned_abs();
            LimitWrite::active_for(watts, left)
        }
        _ => LimitWrite::deactivated(),
    }
}

/// Dial `household`, trust it, and hold it to `reduction` until `silenced` —
/// after which the box goes quiet, heartbeat included.
pub(super) async fn run(
    identity: Identity,
    household: SocketAddr,
    reduction: Option<Reduction>,
    clock: Clock,
    seen: watch::Sender<Seen>,
    silenced: Arc<AtomicBool>,
) {
    let (engine, mut guard) = guard();
    let node = Node::new(SHIP_ID, ShipTls::new(identity), TrustStore::new());
    let mut hub = Hub::new(node, engine);
    let mut connected = false;
    let mut attached = None;
    let mut written: Option<bool> = None;
    let mut redial = core::time::Duration::ZERO;

    loop {
        if silenced.load(Ordering::Relaxed) {
            return;
        }
        let now = hub.now();
        if !connected && now >= redial {
            hub.dial(household);
            redial = now + core::time::Duration::from_secs(3);
        }
        hub.wake_at(guard.poll_timeout());
        hub.wake_at(now + core::time::Duration::from_secs(1));

        let Ok(event) = hub.next().await else {
            tracing::error!("the Steuerbox's hub stopped");
            return;
        };
        let now = hub.now();
        let mut reports = Vec::new();
        match event {
            // A simulator trusts the one household it was pointed at.
            HubEvent::TrustRequested { peer, .. } => hub.approve(peer.ski),
            HubEvent::Connected { ski, .. } => {
                connected = true;
                let _ = hub.remember(ski, household);
                seen.send_modify(|s| s.connected = true);
                tracing::info!("the Steuerbox is connected to the household");
            }
            HubEvent::Disconnected { .. } => {
                connected = false;
                attached = None;
                written = None;
                seen.send_modify(|s| s.connected = false);
            }
            HubEvent::PeerDiscovered { device, .. } => {
                if let Some(peer) = hub
                    .engine()
                    .peer(&device)
                    .and_then(|remote| limitation::locate(remote, lpc::DIRECTION))
                {
                    guard.attach(hub.engine_mut(), peer, now);
                    attached = Some(device);
                    written = None;
                }
            }
            HubEvent::Spine(event) => {
                reports.extend(guard.handle_event(hub.engine_mut(), &event, now));
            }
            HubEvent::Tick => reports = guard.handle_timeout(hub.engine_mut(), now),
            _ => {}
        }

        // Write on each edge of the reduction, and once after every (re)attach.
        let limit = required(reduction, clock.since_midnight());
        if let Some(device) = &attached
            && written != Some(limit.is_active)
        {
            guard.require(device, Some(limit), now);
            written = Some(limit.is_active);
        }

        for report in reports {
            if let GuardEvent::LimitAccepted { limit, .. } = report {
                tracing::info!(
                    watts = limit.watts,
                    active = limit.is_active,
                    "limit accepted"
                );
                seen.send_modify(|s| s.accepted_w = limit.is_active.then_some(limit.watts));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_identity_keeps_its_ski_and_is_the_owners_alone() {
        let path = std::env::temp_dir().join(format!("hems-house-{}.pem", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let first = identity(Some(&path)).expect("issued");
        let again = identity(Some(&path)).expect("read back");
        assert_eq!(
            first.ski, again.ski,
            "the SKI the box's configuration names"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .expect("written")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_limit_is_written_for_what_is_left_of_the_reduction() {
        let teatime = Some((
            Duration::hours(17),
            Duration::minutes(18 * 60 + 30),
            4_200.0,
        ));
        assert!(!required(teatime, Duration::hours(16)).is_active);
        let during = required(teatime, Duration::hours(18));
        assert!(during.is_active);
        assert_eq!(during.duration, Some(core::time::Duration::from_mins(30)));
        assert!(!required(teatime, Duration::minutes(18 * 60 + 30)).is_active);
        assert!(!required(None, Duration::hours(18)).is_active);
    }
}
