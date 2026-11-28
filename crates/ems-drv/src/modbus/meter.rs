//! Commissioned SunSpec model 203 meter. A device qualifies after its common
//! identity, model chain and first complete measurement have been read.

use crate::{Driver, DriverCapabilities, DriverError, DriverEvent, LinkState};
use ems_core::prelude::{AssetId, Measurement};
use ems_core::setpoint::Command;
use std::collections::VecDeque;
use time::OffsetDateTime;

use super::frame::{self, Request, RequestBody, ResponseBody, Space};
use super::probe::{Identity, Probe};
use super::{Cadence, decode};

/// Read-only three-phase wye meter, shared by TCP and RTU clients.
pub struct Meter {
    asset: AssetId,
    unit: u8,
    cadence: Cadence,
    probe: Probe,
    link: LinkState,
    transaction: u16,
    pending: Option<Request>,
    outbox: Option<Vec<u8>>,
    inbox: Vec<u8>,
    due: Option<OffsetDateTime>,
    events: VecDeque<DriverEvent>,
}

impl Meter {
    /// Commission a single unit. Link activation starts model discovery.
    pub fn new(asset: AssetId, unit: u8, cadence: Cadence) -> Self {
        Self {
            asset,
            unit,
            cadence,
            probe: Probe::new(unit),
            link: LinkState::Down,
            transaction: 0,
            pending: None,
            outbox: None,
            inbox: Vec::new(),
            due: None,
            events: VecDeque::new(),
        }
    }

    fn set_link(&mut self, state: LinkState) {
        if self.link != state {
            self.link = state;
            self.events.push_back(DriverEvent::Link(state));
        }
    }

    fn next_read(&mut self, now: OffsetDateTime) {
        let request = self.probe.request().or_else(|| {
            let (address, count) = self.probe.meter_block()?;
            Some(Request {
                unit: self.unit,
                transaction: 0,
                body: RequestBody::Read {
                    space: Space::Holding,
                    address,
                    count,
                },
            })
        });
        if let Some(mut request) = request {
            self.transaction = self.transaction.wrapping_add(1);
            request.transaction = self.transaction;
            self.outbox = Some(request.encode());
            self.pending = Some(request);
            self.due = Some(now + self.cadence.timeout);
        } else {
            self.on_link(LinkState::Stale, now);
        }
    }

    fn receive(&mut self, reply: &frame::Response, now: OffsetDateTime) -> Result<(), DriverError> {
        let Some(request) = &self.pending else {
            return Ok(());
        };
        if reply.transaction != request.transaction || reply.unit != self.unit {
            return Ok(());
        }
        let RequestBody::Read { count, .. } = request.body else {
            unreachable!()
        };
        if reply.function != frame::function::READ_HOLDING
            || matches!(&reply.body, ResponseBody::Registers(words) if words.len() != usize::from(count))
        {
            return Err(DriverError::Malformed(
                "unexpected SunSpec meter reply".into(),
            ));
        }
        self.pending = None;
        if self.probe.meter_block().is_none() {
            self.probe.answer(&reply.body);
            self.next_read(now);
        } else {
            let mut reading = Measurement::at(now);
            if let ResponseBody::Registers(words) = &reply.body {
                decode::fold(203, words, &mut reading);
            }
            if reading.power.is_none()
                || reading.energy_in.is_none()
                || reading.energy_out.is_none()
            {
                return Err(DriverError::Malformed(
                    "unusable SunSpec meter reading".into(),
                ));
            }
            self.set_link(LinkState::Up);
            self.events.push_back(DriverEvent::Measured(reading));
            self.due = Some(now + self.cadence.poll);
        }
        Ok(())
    }
}

impl Driver for Meter {
    fn asset(&self) -> &AssetId {
        &self.asset
    }

    fn meter_identity(&self) -> Option<&Identity> {
        self.probe
            .found()
            .filter(|identity| identity.connection_meter)
    }

    fn capabilities(&self) -> DriverCapabilities {
        DriverCapabilities::meter()
    }

    fn on_link(&mut self, state: LinkState, now: OffsetDateTime) {
        self.pending = None;
        self.outbox = None;
        self.inbox.clear();
        self.probe = Probe::new(self.unit);
        match state {
            LinkState::Connecting | LinkState::Up => {
                self.set_link(LinkState::Connecting);
                self.next_read(now);
            }
            LinkState::Down | LinkState::Stale => {
                self.set_link(state);
                self.due = (state == LinkState::Stale).then(|| now + self.cadence.poll);
            }
        }
    }

    fn on_bytes(&mut self, bytes: &[u8], now: OffsetDateTime) -> Result<(), DriverError> {
        // Only one read is in flight. A TCP header and one PDU fit in 260 bytes.
        if bytes.len() > 260 - self.inbox.len() {
            self.on_link(LinkState::Stale, now);
            return Err(DriverError::Malformed(
                "oversized SunSpec meter reply".into(),
            ));
        }
        self.inbox.extend_from_slice(bytes);
        let result = match frame::decode(&self.inbox) {
            Ok(None) => return Ok(()),
            Ok(Some((reply, used))) if used == self.inbox.len() => {
                self.inbox.clear();
                self.receive(&reply, now)
            }
            Ok(Some(_)) => Err(DriverError::Malformed("extra SunSpec meter reply".into())),
            Err(error) => Err(DriverError::Malformed(error.to_string())),
        };
        if result.is_err() {
            self.on_link(LinkState::Stale, now);
        }
        result
    }

    fn on_timeout(&mut self, now: OffsetDateTime) {
        if self.due.is_some_and(|at| at <= now) {
            if self.pending.is_some() {
                self.on_link(LinkState::Stale, now);
            } else {
                self.next_read(now);
            }
        }
    }

    fn command(&mut self, _: &Command, _: OffsetDateTime) -> Result<(), DriverError> {
        Err(DriverError::Unsupported(
            "the grid meter is read-only".into(),
        ))
    }

    fn poll_event(&mut self) -> Option<DriverEvent> {
        self.events.pop_front()
    }
    fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        self.outbox.take()
    }
    fn poll_deadline(&self) -> Option<OffsetDateTime> {
        self.due
    }
}
