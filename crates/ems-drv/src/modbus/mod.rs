//! SunSpec and vendor register drivers for Modbus TCP and read-only RTU meters.
//!
//! [`Meter`] commissions a three-phase wye meter using its model chain. [`rtu`]
//! carries the driver's register reads over a serial line. [`SunSpec`] handles
//! general device discovery and control. The host supplies the transport and clock.
//!
//! Inverter model 701 supplies throttling observations for a labelled estimate
//! of available power during curtailment. Models 101–103 report production.

mod decode;
pub mod frame;
mod meter;
pub mod probe;
pub mod registers;
pub mod rtu;
mod scan;
mod storage;

pub use meter::Meter;
pub use scan::{Discovery, ModelMap};

use crate::{CommandOutcome, Driver, DriverCapabilities, DriverError, DriverEvent, LinkState};
use ems_core::prelude::{AssetId, Measurement, Power};
use ems_core::setpoint::Command;
use time::{Duration, OffsetDateTime};

use frame::{Request, RequestBody, Response, ResponseBody};

/// What kind of thing is on the other end.
///
/// Decided by which models the device publishes rather than by configuration: a
/// device that carries model 103 is an inverter whatever a TOML file calls it,
/// and a mismatch between the two is worth finding at discovery rather than in a
/// measurement that reads plausibly and means something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A photovoltaic inverter — models 101/102/103, or 701.
    Inverter,
    /// A meter — models 201–204.
    Meter,
    /// A battery — model 802.
    Battery,
}

/// How often to poll, and how long to wait for an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cadence {
    /// How often a full read of the interesting models is issued.
    ///
    /// A second is what the guard's control period wants; a slow inverter over a
    /// serial gateway may need more. It is a *floor*: the driver does not start
    /// a new poll while one is outstanding, so a device that answers in two
    /// seconds is polled every two.
    pub poll: Duration,
    /// How long an unanswered request may stand before the link is stale.
    ///
    /// Not a retry: this is the point at which the guard should stop believing
    /// the last measurement. It has to be shorter than the guard's own
    /// `max_measurement_age`, or a device could be silent for a whole control
    /// period while the guard still counted its last reading as fresh.
    pub timeout: Duration,
}

impl Default for Cadence {
    fn default() -> Self {
        Self {
            poll: Duration::seconds(1),
            timeout: Duration::seconds(5),
        }
    }
}

/// A SunSpec device on the other end of a Modbus TCP connection.
#[derive(Debug)]
pub struct SunSpec {
    asset: AssetId,
    unit: u8,
    cadence: Cadence,
    /// Where the model list starts. SunSpec allows three, and a device answers
    /// on exactly one of them.
    discovery: Discovery,
    models: ModelMap,
    kind: Option<Kind>,
    link: LinkState,
    /// Bytes that have arrived and are not yet a whole frame.
    inbox: Vec<u8>,
    /// Requests waiting to go out.
    outbox: Vec<Request>,
    /// What was asked, and when — so an answer can be matched and a silence
    /// noticed.
    pending: Option<Pending>,
    next_transaction: u16,
    events: Vec<DriverEvent>,
    /// When the next poll is due.
    due: Option<OffsetDateTime>,
    /// The measurement being assembled from several models this round.
    partial: Option<Measurement>,
    /// Whether this driver is configured to read and never command.
    listens_only: bool,
    /// The registers of the model being polled, assembled across reads: one
    /// request carries at most [`frame::MAX_REGISTERS_PER_READ`], and model 701
    /// alone is longer than that.
    block: Vec<u16>,
    /// A ceiling waiting for the device's `WMaxLimPct_SF` before it can be
    /// written.
    awaiting_scale: Option<Power>,
    storage: Option<storage::Control>,
}

/// A request that has gone out and not yet come back.
#[derive(Debug, Clone)]
struct Pending {
    transaction: u16,
    at: OffsetDateTime,
    purpose: Purpose,
}

/// Why a request was sent, so its answer can be understood.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// Battery target write and verification use one outstanding transaction.
    StorageWrite { address: u16, target: Option<i32> },
    StorageReadback {
        target: Option<i32>,
        sent: OffsetDateTime,
    },
    /// Looking for the `SunS` marker at one of the three base addresses.
    Marker(u16),
    /// Reading a model header while walking the chain.
    Header(u16),
    /// Reading the models that carry a measurement: model `id`, whose body of
    /// `len` registers starts at `address`.
    Poll { id: u16, address: u16, len: u16 },
    /// Reading model 123's `WMaxLimPct_SF`, which a ceiling is scaled by.
    Scale,
    /// Writing a curtailment setpoint.
    Curtail { address: u16, wanted: u16 },
    /// Reading back a curtailment setpoint the device has just accepted.
    ///
    /// A Modbus write response echoes the address and the quantity and **not
    /// the values**, so "accepted" means the frame was well formed and nothing
    /// more. A device that clipped the percentage to its own range, or that
    /// took `WMaxLimPct` and left `WMaxLim_Ena` at zero, answers a write
    /// exactly as one that obeyed. The only way to know which happened is to
    /// read the registers back.
    Readback { address: u16, wanted: u16 },
}

impl SunSpec {
    /// Read this device and never command it.
    ///
    /// A meter is the ordinary case, and so is an inverter behind a vendor
    /// gateway that exposes SunSpec read-only. The distinction is *site
    /// configuration* rather than something to discover: the alternative is a
    /// driver that claims it can command until a model walk says otherwise,
    /// which is a claim nothing can check at the moment it matters.
    #[must_use]
    pub fn listening_only(mut self) -> Self {
        self.listens_only = true;
        self
    }

    /// The device's rated power, which curtailment is expressed against.
    ///
    /// SunSpec model 123 curtails in **per cent of `WMax`**, so a ceiling in
    /// watts cannot be written without it. It comes from the site rather than
    /// from the wire because the site already knows it — an `Asset` carries its
    /// connection power — and because the nameplate models that would publish it
    /// are optional, so a driver that only read them would refuse to curtail
    /// perfectly ordinary inverters.
    ///
    /// Without it [`Driver::command`] refuses a production ceiling rather than
    /// writing a percentage computed from a guess.
    #[must_use]
    pub fn with_rating(mut self, rating: Power) -> Self {
        self.models.set_rating(rating);
        self
    }

    /// A device at `unit` behind a Modbus TCP connection.
    pub fn new(asset: AssetId, unit: u8, cadence: Cadence) -> Self {
        Self {
            asset,
            unit,
            cadence,
            discovery: Discovery::new(),
            models: ModelMap::default(),
            kind: None,
            link: LinkState::Down,
            inbox: Vec::new(),
            outbox: Vec::new(),
            pending: None,
            next_transaction: 1,
            events: Vec::new(),
            due: None,
            partial: None,
            listens_only: false,
            block: Vec::new(),
            awaiting_scale: None,
            storage: None,
        }
    }

    /// What the device turned out to be, once discovery has finished.
    pub fn kind(&self) -> Option<Kind> {
        self.kind
    }

    /// Which models the device publishes.
    pub fn models(&self) -> &ModelMap {
        &self.models
    }

    /// The next transaction identifier.
    ///
    /// Wraps, and skips zero: some gateways treat a zero transaction as "no
    /// correlation" and will answer it out of order.
    fn transaction(&mut self) -> u16 {
        self.next_transaction = self.next_transaction.wrapping_add(1).max(1);
        self.next_transaction
    }

    /// Queue a read.
    fn read(&mut self, address: u16, count: u16, purpose: Purpose, now: OffsetDateTime) {
        let transaction = self.transaction();
        self.outbox.push(Request {
            transaction,
            unit: self.unit,
            body: RequestBody::Read {
                space: frame::Space::Holding,
                address,
                count,
            },
        });
        self.pending = Some(Pending {
            transaction,
            at: now,
            purpose,
        });
    }

    /// Take the next step of whatever the driver is in the middle of.
    fn advance(&mut self, now: OffsetDateTime) {
        if self.pending.is_some() {
            return;
        }
        // Discovery first: nothing can be read until the model list is known.
        if let Some((address, count, purpose)) = self.discovery.next_step(&self.models) {
            self.read(address, count, purpose, now);
            return;
        }
        self.kind = Some(self.models.kind());
        if self.storage.is_none() && matches!(self.models.next_poll(), Some((704, _, _))) {
            self.models.advance_poll();
        }
        if self.link != LinkState::Up {
            self.link = LinkState::Up;
            self.events.push(DriverEvent::Link(LinkState::Up));
        }
        // Then the poll, when it is due.
        if self.due.is_none_or(|due| now >= due) {
            if let Some((id, address, len)) = self.models.next_poll() {
                self.partial.get_or_insert_with(|| Measurement::at(now));
                self.block.clear();
                self.read(
                    address,
                    len.min(frame::MAX_REGISTERS_PER_READ),
                    Purpose::Poll { id, address, len },
                    now,
                );
            } else {
                self.finish_poll(now);
            }
        }
    }

    /// Emit whatever the round assembled and schedule the next one.
    fn finish_poll(&mut self, now: OffsetDateTime) {
        if let Some(measurement) = self.partial.take() {
            self.events.push(DriverEvent::Measured(measurement));
        }
        self.models.rewind();
        self.due = Some(now + self.cadence.poll);
    }
}

impl Driver for SunSpec {
    fn asset(&self) -> &AssetId {
        &self.asset
    }

    fn storage_feedback(&self, now: OffsetDateTime) -> Option<crate::StorageFeedback> {
        self.storage_feedback(now)
    }

    fn can_limit_production(&self) -> bool {
        !self.listens_only
            && self.kind.is_some()
            && self.models.has(123)
            && self.models.rating().is_some()
            && self.models.curtail_scale_address().is_some()
    }

    fn capabilities(&self) -> DriverCapabilities {
        // What the driver is **for**, not what it has discovered. Registration
        // happens before the first byte, so a capability that only became true
        // after a model walk would be a capability nothing could check — and the
        // check is the whole reason the type exists. The role comes from the
        // site, which already knows whether this address is an inverter or a
        // meter, and discovery is not consulted about it.
        let base = if self.listens_only {
            DriverCapabilities::meter()
        } else {
            DriverCapabilities::device()
        };
        // Only a device that publishes model 701 can say what it *could*
        // produce; see the module note. Guessing here would be the difference
        // between a curtailment that lifts and one that does not.
        if self.models.kind() == Kind::Inverter && self.models.reports_available_power() {
            base.with_available_power()
        } else {
            base
        }
    }

    fn on_bytes(&mut self, bytes: &[u8], now: OffsetDateTime) -> Result<(), DriverError> {
        self.inbox.extend_from_slice(bytes);
        loop {
            match frame::decode(&self.inbox) {
                Ok(None) => break,
                Ok(Some((response, used))) => {
                    self.inbox.drain(..used);
                    self.handle(&response, now);
                }
                Err(e) => {
                    // A frame this driver cannot parse means the stream is no
                    // longer aligned; keeping the rest would decode rubbish at
                    // an offset. Dropping it is the only honest recovery.
                    self.inbox.clear();
                    self.on_link(LinkState::Stale, now);
                    return Err(DriverError::Malformed(e.to_string()));
                }
            }
        }
        self.advance(now);
        Ok(())
    }

    /// A socket opened or closed under this driver.
    ///
    /// Everything a *session* owns is dropped: the half-frame in the inbox, the
    /// request waiting for an answer, the queue of requests that were never
    /// sent, and the half-assembled measurement they were building. Keeping any
    /// of them across a reconnect is how a box comes back from a twenty-second
    /// outage and decodes the new stream at the old one's offset.
    ///
    /// The **discovery** is dropped with them, and that is not caution for its
    /// own sake: an address that reconnects may be a different device — a
    /// replaced inverter, a Modbus gateway that renumbered its units, a DHCP
    /// lease that moved. Walking the chain again costs three reads.
    fn on_link(&mut self, state: LinkState, now: OffsetDateTime) {
        if let Some(p) = self.pending.clone() {
            self.abandon_command(p.purpose, "the link went before it was answered", now);
        }
        self.inbox.clear();
        self.outbox.clear();
        self.pending = None;
        self.partial = None;
        self.block.clear();
        self.awaiting_scale = None;
        self.models.restart();
        if let Some(s) = &mut self.storage {
            s.reset();
        }
        self.kind = None;
        self.due = None;
        self.discovery.restart();
        if self.link != state {
            self.link = state;
            self.events.push(DriverEvent::Link(state));
        }
        if state == LinkState::Up {
            // The first read of the new session goes out now rather than at the
            // next poll: a reconnect that then waited a cadence would leave the
            // guard assuming a nameplate for a device that is answering.
            self.advance(now);
        }
    }

    fn on_timeout(&mut self, now: OffsetDateTime) {
        // An unanswered request is the only thing a timeout means here. The
        // guard is told the link is stale rather than being left with a
        // measurement that looks fresh because nothing contradicted it.
        if let Some(p) = self.pending.clone()
            && now - p.at >= self.cadence.timeout
        {
            self.abandon_command(p.purpose, "the device did not answer", now);
            self.pending = None;
            self.partial = None;
            self.block.clear();
            self.awaiting_scale = None;
            if self.link != LinkState::Stale {
                self.link = LinkState::Stale;
                self.events.push(DriverEvent::Link(LinkState::Stale));
            }
            self.discovery.restart();
            self.models.restart();
            if let Some(s) = &mut self.storage {
                s.reset();
            }
            self.kind = None;
            self.outbox.clear();
            self.inbox.clear();
        }
        self.advance(now);
    }

    fn command(&mut self, command: &Command, now: OffsetDateTime) -> Result<(), DriverError> {
        if self.listens_only {
            return Err(DriverError::Unsupported(
                "this SunSpec driver is read-only".into(),
            ));
        }
        if self.storage.is_some() {
            return self.command_storage(command, now);
        }
        let Command::ProductionCeiling(ceiling) = command else {
            return Err(DriverError::Unsupported(format!("{command:?}")));
        };
        // The link first, and the order matters. A device that has never been
        // connected to has never been walked either, so it has no model 123
        // *yet* — answering "this device cannot be curtailed" about a device
        // nobody has spoken to is a diagnosis of the wrong thing, and it is the
        // one an operator would act on.
        if self.link != LinkState::Up || self.kind.is_none() {
            return Err(DriverError::NotConnected);
        }
        // Refused here, before anything is sent, where the device has no
        // model 123 or the driver no rating: the caller learns it at once.
        if !self.models.has(123) || self.models.rating().is_none() {
            return Err(DriverError::Unsupported(
                "a production ceiling on a device with no model 123 or no rating".into(),
            ));
        }
        // `WMaxLimPct` is stored with the device's own scale factor, and a
        // percentage written without it is wrong by a power of ten: sixty per
        // cent sent as `60` to a device with `WMaxLimPct_SF = −2` is 0,6 %.
        // It is read once per session, before the first ceiling.
        if self.models.curtail_scale().is_none() {
            let Some(address) = self.models.curtail_scale_address() else {
                return Err(DriverError::Unsupported(
                    "model 123 lies outside the address space".into(),
                ));
            };
            self.awaiting_scale = Some(*ceiling);
            self.read(address, 1, Purpose::Scale, now);
            return Ok(());
        }
        if self.write_ceiling(*ceiling, now) {
            Ok(())
        } else {
            Err(DriverError::Unsupported(
                "a production ceiling this device's scale factor cannot express".into(),
            ))
        }
    }

    fn poll_event(&mut self) -> Option<DriverEvent> {
        if self.events.is_empty() {
            None
        } else {
            Some(self.events.remove(0))
        }
    }

    fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        if self.outbox.is_empty() {
            None
        } else {
            Some(self.outbox.remove(0).encode())
        }
    }

    fn poll_deadline(&self) -> Option<OffsetDateTime> {
        // Only one request can be in flight. An overdue poll cannot run until
        // the current reply arrives or times out.
        self.pending
            .as_ref()
            .map(|p| p.at + self.cadence.timeout)
            .or(self.due)
    }
}

impl SunSpec {
    /// Queue the write of a ceiling. `false` where it cannot be expressed.
    fn write_ceiling(&mut self, ceiling: Power, now: OffsetDateTime) -> bool {
        let Some(write) = self.models.curtailment_write(ceiling) else {
            return false;
        };
        let transaction = self.transaction();
        let address = write.address;
        let wanted = write.values[0];
        self.outbox.push(Request {
            transaction,
            unit: self.unit,
            body: RequestBody::Write {
                address,
                values: write.values,
            },
        });
        self.pending = Some(Pending {
            transaction,
            at: now,
            purpose: Purpose::Curtail { address, wanted },
        });
        true
    }

    fn fold_poll(&mut self, id: u16, address: u16, len: u16, regs: &[u16], now: OffsetDateTime) {
        self.block.extend_from_slice(regs);
        let have = u16::try_from(self.block.len()).unwrap_or(u16::MAX);
        // A model longer than one read is fetched in pieces and folded
        // whole. An empty answer ends it: a device that returns nothing
        // would otherwise be asked for the same registers for ever.
        if let (true, false, Some(next)) = (have < len, regs.is_empty(), address.checked_add(have))
        {
            self.read(
                next,
                (len - have).min(frame::MAX_REGISTERS_PER_READ),
                Purpose::Poll { id, address, len },
                now,
            );
        } else {
            let block = core::mem::take(&mut self.block);
            if let Some(m) = self.partial.as_mut() {
                decode::fold(id, &block, m);
                if id == 704
                    && let Some(s) = &mut self.storage
                {
                    s.observe(&block, now);
                }
            }
            self.models.advance_poll();
        }
    }

    /// Fold one answer into whatever it was asked for.
    fn handle(&mut self, response: &Response, now: OffsetDateTime) {
        let Some(pending) = self.pending.clone() else {
            // An answer to nothing: a late reply to a request already given up
            // on. Dropping it is right — folding it in would put a stale reading
            // into a fresh measurement.
            return;
        };
        if response.unit != self.unit || pending.transaction != response.transaction {
            return;
        }
        self.pending = None;

        if self.handle_storage(&pending, response, now) {
            return;
        }
        match (&pending.purpose, &response.body) {
            // A device that refuses the write, or refuses to be read back, has
            // not been curtailed. It is a *command* answer and not a discovery
            // one: routing it through `refused` would end the model walk on the
            // strength of an inverter that simply does not accept a setpoint.
            (
                Purpose::Curtail { .. } | Purpose::Readback { .. } | Purpose::Scale,
                ResponseBody::Exception { code, .. },
            ) => {
                self.awaiting_scale = None;
                self.events.push(DriverEvent::Command(CommandOutcome {
                    accepted: false,
                    confirmed: None,
                    at: now,
                    detail: Some(format!("the device refused it, exception {code}")),
                }));
            }
            (Purpose::Poll { id, .. }, ResponseBody::Exception { .. }) => {
                if *id == 704
                    && let Some(s) = &mut self.storage
                {
                    s.observe(&[], now);
                }
                self.models.advance_poll();
            }
            // An exception while walking is how a device says "no model there".
            (_, ResponseBody::Exception { .. }) => {
                self.discovery.refused(pending.purpose, &mut self.models);
            }
            (Purpose::Marker(base), ResponseBody::Registers(regs)) => {
                self.discovery.saw_marker(*base, regs, &mut self.models);
            }
            (Purpose::Header(address), ResponseBody::Registers(regs)) => {
                self.discovery.saw_header(*address, regs, &mut self.models);
            }
            (Purpose::Poll { id, address, len }, ResponseBody::Registers(regs))
                if response.function == 3 =>
            {
                self.fold_poll(*id, *address, *len, regs, now);
            }
            (Purpose::Scale, ResponseBody::Registers(regs)) if regs.len() == 1 => {
                // An unimplemented scale factor reads `0x8000`, which `scaled`
                // treats as no scaling.
                self.models.set_curtail_scale(regs[0].cast_signed());
                if let Some(ceiling) = self.awaiting_scale.take()
                    && !self.write_ceiling(ceiling, now)
                {
                    self.events.push(DriverEvent::Command(CommandOutcome {
                        accepted: false,
                        confirmed: None,
                        at: now,
                        detail: Some(
                            "the device's WMaxLimPct_SF cannot express this ceiling".into(),
                        ),
                    }));
                }
            }
            (
                Purpose::Curtail { address, wanted },
                ResponseBody::WriteAccepted {
                    address: echoed,
                    count,
                },
            ) if address == echoed && *count == CURTAIL_REGISTERS => {
                // Nothing is reported yet. The write went out well formed,
                // which is all this answer says; the outcome is what comes back
                // off the registers.
                let (address, wanted) = (*address, *wanted);
                self.read(
                    address,
                    CURTAIL_REGISTERS,
                    Purpose::Readback { address, wanted },
                    now,
                );
            }
            (Purpose::Readback { wanted, .. }, ResponseBody::Registers(regs)) => {
                let outcome = self.readback(*wanted, regs, now);
                self.events.push(DriverEvent::Command(outcome));
            }
            _ => {
                self.awaiting_scale = None;
                self.abandon_command(
                    pending.purpose,
                    "unexpected Modbus response to the control request",
                    now,
                );
            }
        }
    }

    /// Report a curtailment that will never be answered, and nothing otherwise.
    ///
    /// A command whose answer never arrives is not a command that succeeded,
    /// and it is not one that is still in flight either — the request has been
    /// dropped. Saying so is what keeps `[A1 7.2]` a record of what was
    /// commanded *and confirmed*: silence at this seam reaches the evidence as
    /// a ceiling that was issued and never contradicted, which reads as
    /// compliance. A discovery read that goes unanswered is a different thing
    /// and stays where it is, in the link state.
    fn abandon_command(&mut self, purpose: Purpose, why: &str, now: OffsetDateTime) {
        if matches!(
            purpose,
            Purpose::Curtail { .. }
                | Purpose::Readback { .. }
                | Purpose::Scale
                | Purpose::StorageWrite { .. }
                | Purpose::StorageReadback { .. }
        ) {
            self.events.push(DriverEvent::Command(CommandOutcome {
                accepted: false,
                confirmed: None,
                at: now,
                detail: Some(why.to_owned()),
            }));
        }
    }

    /// What the device says it will actually do, read off its own registers.
    ///
    /// Two things can have gone wrong and neither shows in the write answer.
    /// The device may have **clipped** the percentage — an inverter with a
    /// minimum curtailment step, or one that refuses to go below some floor —
    /// which is a smaller reduction than the guard commanded and a number the
    /// plan needs. Or it may have taken `WMaxLimPct` and left `WMaxLim_Ena` at
    /// zero, which is the classic way to curtail nothing at all and believe
    /// otherwise: the setpoint is stored, the limit is not in force, and every
    /// layer above reports a compliant house until the meter disagrees.
    ///
    /// A short answer is not read optimistically. A device that returned fewer
    /// registers than the model has is one whose enable flag was not in the
    /// reply, and assuming it is set would be inventing the confirmation this
    /// whole exchange exists to obtain.
    fn readback(&self, wanted: u16, regs: &[u16], now: OffsetDateTime) -> CommandOutcome {
        let Some((&raw, &enabled)) = regs.first().zip(regs.get(ENABLE_OFFSET)) else {
            return CommandOutcome {
                accepted: false,
                confirmed: None,
                at: now,
                detail: Some(format!(
                    "the read-back returned {} registers, too few to see WMaxLim_Ena",
                    regs.len()
                )),
            };
        };
        let in_force = enabled == 1;
        let scale = self.models.curtail_scale().unwrap_or(0);
        let percent = |raw: u16| decode::scaled(i64::from(raw), scale);
        let confirmed = self
            .models
            .rating()
            .map(|rating| Power::new(rating.get() * percent(raw) / 100.0));
        let detail = if !in_force {
            Some("WMaxLimPct was stored but WMaxLim_Ena reads 0: the limit is not in force".into())
        } else if raw != wanted {
            Some(format!(
                "the device clipped {} % to {} %",
                percent(wanted),
                percent(raw)
            ))
        } else {
            None
        };
        CommandOutcome {
            accepted: in_force,
            confirmed,
            at: now,
            detail,
        }
    }
}

/// How many registers a curtailment occupies: `WMaxLimPct` through
/// `WMaxLim_Ena`.
const CURTAIL_REGISTERS: u16 = 5;

/// Where `WMaxLim_Ena` sits in them.
const ENABLE_OFFSET: usize = 4;

/// A curtailment write, resolved to registers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurtailWrite {
    /// The first register.
    pub address: u16,
    /// What to put there.
    pub values: Vec<u16>,
}

/// What a ceiling means in `WMaxLimPct`'s own units.
///
/// SunSpec model 123 curtails in **per cent of `WMax`**, not in watts, so a
/// driver that does not know the rating cannot express a ceiling at all — and
/// saying so is better than writing a percentage computed from a guess. The
/// register holds the percentage divided by `10^scale` (`WMaxLimPct_SF`): sixty
/// per cent is `60` at a scale of 0 and `6000` at −2. `None` where the rating
/// is not positive or the result does not fit the register.
#[must_use]
pub fn curtail_register(ceiling: Power, rating: Power, scale: i16) -> Option<u16> {
    if rating <= Power::ZERO {
        return None;
    }
    let pct = (ceiling.get() / rating.get() * 100.0).clamp(0.0, 100.0);
    let unit = decode::scaled(1, scale);
    let raw = (pct / unit).round();
    if !(0.0..=f64::from(u16::MAX)).contains(&raw) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked to lie in a u16's range on the line above, and rounded"
    )]
    Some(raw as u16)
}
