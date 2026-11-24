//! Sans-I/O device and grid drivers for the EMS edge systems.
//!
//! The caller supplies frames and timestamps and handles transport. Device
//! drivers publish [`DriverEvent::Measured`] and accept device commands.
//! Grid drivers publish [`DriverEvent::GridLimit`]. [`DriverCapabilities`]
//! describes what the registry can accept for a site.
//!
//! Enable `modbus` or `eebus` for household drivers, `ocpp` for charging-station
//! evidence or `matter` for appliance power mapping.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::doc_markdown,
    clippy::cast_precision_loss
)]

#[cfg(feature = "ocpp")]
pub mod ocpp;

#[cfg(feature = "matter")]
pub mod matter;

mod capability;
mod event;
mod link;

#[cfg(feature = "eebus")]
pub mod eebus;

#[cfg(feature = "eebus")]
pub mod eebus_dhw;

#[cfg(feature = "eebus")]
pub mod eebus_ev;

#[cfg(feature = "eebus")]
pub mod eebus_heat_pump;
#[cfg(feature = "modbus")]
pub mod modbus;

pub use capability::DriverCapabilities;
pub use event::{
    CommandOutcome, DriverEvent, Failsafe, FeedInFactor, Flexibility, GridLimit, LimitDirection,
    LimitSource, VehiclePresence,
};
pub use link::LinkState;

use ems_core::prelude::AssetId;
use ems_core::setpoint::Command;
use time::OffsetDateTime;

/// Why a driver could not do what it was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DriverError {
    /// The bytes were not a frame this protocol recognises.
    ///
    /// Recoverable: `hemsd` logs it and keeps the link. A device that emits one
    /// malformed frame an hour is a device, not an outage.
    #[error("malformed frame: {0}")]
    Malformed(String),
    /// The device answered, and the answer was a refusal.
    #[error("the device refused: {0}")]
    Refused(String),
    /// The command is one this driver cannot express on this protocol.
    ///
    /// A *programming* error rather than a runtime one: the arbiter should not
    /// have produced it, because [`DriverCapabilities`] said the device could
    /// not take it. Worth an error rather than a silent drop, because a command
    /// nobody sends and nobody reports is how a device quietly stops being
    /// managed.
    #[error("this driver cannot express {0}")]
    Unsupported(String),
    /// The link is not in a state where anything can be sent.
    #[error("not connected")]
    NotConnected,
    /// Nothing speaks for this asset at all.
    ///
    /// Distinct from [`DriverError::Unsupported`], and the distinction is what
    /// keeps a log readable: "this device cannot do that" is a fault to look
    /// into, and "this device has no driver" is a **configuration** fact that is
    /// equally true on every tick. Reported as the same error, a partially
    /// commissioned box writes one warning per asset per second for ever, and
    /// the real fault is somewhere in the middle of it.
    #[error("no driver speaks for {0}")]
    NoDriver(String),
}

/// The contract every driver keeps.
///
/// # The order `hemsd` calls these in
///
/// ```text
/// socket = connect()?;                       driver.on_link(Up, now);
/// loop {
///     // whichever comes first
///     bytes = read(socket, until: driver.poll_deadline())
///     match bytes {
///         Some(b)  => driver.on_bytes(&b, now)?,
///         None     => driver.on_timeout(now),
///         Closed   => { driver.on_link(Down, now); break }
///     }
///     while let Some(event) = driver.poll_event() { … }
///     while let Some(out)   = driver.poll_transmit() { write(socket, out) }
/// }
/// ```
///
/// Nothing here blocks and nothing here allocates a runtime. A driver that
/// needs to wait says so with [`Driver::poll_deadline`] and is called back.
pub trait Driver {
    /// Confirmed battery control state, where the device implements bounded dispatch.
    fn storage_feedback(&self, _now: OffsetDateTime) -> Option<StorageFeedback> {
        None
    }

    /// Which asset this driver speaks for.
    ///
    /// One driver, one asset. A device that presents several — a hybrid
    /// inverter with a battery behind it — is several drivers over one
    /// transport, because the guard bounds assets and not boxes.
    fn asset(&self) -> &AssetId;

    /// What this driver can report and what it can be told.
    ///
    /// Read once at registration. `hemsd` refuses a site whose configuration
    /// asks a driver for something it has not declared, which is the cheap half
    /// of the bug where a command is sent for a year and silently ignored.
    fn capabilities(&self) -> DriverCapabilities;

    /// Whether this connected device exposes a usable production-ceiling control.
    fn can_limit_production(&self) -> bool {
        false
    }

    /// Last heartbeat that the consumption limitation actor accepted from its Energy Guard.
    fn last_lpc_heartbeat(&self) -> Option<OffsetDateTime> {
        None
    }

    /// The transport under this driver opened or closed.
    ///
    /// # Why a byte-oriented contract still needs this
    ///
    /// A driver holds state that belongs to a *session* and not to a device:
    /// half a Modbus frame that has arrived and is not yet whole, a request that
    /// went out and is waiting for its answer, a SPINE peer it has discovered.
    /// A reconnect invalidates every one of them, and nothing in a stream of
    /// bytes says so — the first bytes of the new socket look exactly like the
    /// continuation of the old one, and a partial frame left over from before
    /// the drop makes the whole stream decode at an offset.
    ///
    /// So the layer that owns the socket says. It is the one fact only that
    /// layer knows, and leaving it out is how a box comes back from a
    /// twenty-second outage reading rubbish and reporting it as a measurement.
    ///
    /// The default is to do nothing, because a stateless driver genuinely has
    /// nothing to forget.
    fn on_link(&mut self, state: LinkState, now: OffsetDateTime) {
        let _ = (state, now);
    }

    /// Bytes arrived from the device.
    ///
    /// # Errors
    /// [`DriverError::Malformed`] when they are not a frame. The link survives;
    /// it is the caller's decision whether a rate of them is an outage.
    fn on_bytes(&mut self, bytes: &[u8], now: OffsetDateTime) -> Result<(), DriverError>;

    /// The deadline passed and nothing arrived.
    ///
    /// This is where a heartbeat is missed and a failsafe is entered, so it is
    /// **not** optional and it is not an error path: for a grid driver it is the
    /// most important call in the trait.
    fn on_timeout(&mut self, now: OffsetDateTime);

    /// Ask the device to do something.
    ///
    /// # Errors
    /// [`DriverError::Unsupported`] where [`Driver::capabilities`] said so, and
    /// [`DriverError::NotConnected`] where the link is down. Neither is a reason
    /// to drop the command silently.
    fn command(&mut self, command: &Command, now: OffsetDateTime) -> Result<(), DriverError>;

    /// The next thing that happened, if anything has.
    ///
    /// Drained to empty each turn. Events are in the order they occurred,
    /// because a limit and the acknowledgement of it are a sequence and a
    /// consumer that reordered them would write the wrong Nachweis.
    fn poll_event(&mut self) -> Option<DriverEvent>;

    /// The next bytes to put on the wire, if any.
    fn poll_transmit(&mut self) -> Option<Vec<u8>>;

    /// When to call [`Driver::on_timeout`] if nothing arrives before then.
    ///
    /// `None` means "only when bytes arrive". A driver with a heartbeat should
    /// never return `None`: that is what makes the failsafe reachable.
    fn poll_deadline(&self) -> Option<OffsetDateTime>;
}

/// Last bounded storage setting confirmed by the device.
#[derive(Debug, Clone, Copy, Default)]
pub struct StorageFeedback {
    /// A write or its verification read is outstanding on the current link.
    pub pending: bool,
    /// Fresh readback shows external watt control is disabled.
    pub autonomous: bool,
    /// Current confirmed target, in household load convention.
    pub confirmed: Option<ems_core::prelude::Power>,
    /// Most recent matched command readback, including explicit release.
    pub last_confirmed: Option<OffsetDateTime>,
}
