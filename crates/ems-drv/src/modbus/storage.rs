//! Commissioned model 704 watt-mode profile used by the virtual house.
use super::{
    Command, CommandOutcome, DriverError, DriverEvent, Kind, LinkState, Pending, Power, Purpose,
    Request, RequestBody, Response, ResponseBody, SunSpec,
};
use sunspec::models::model704::{
    Model704 as M, PfwAbs, PfwAbsRvrt, PfwInj, PfwInjRvrt, WSetEna, WSetEnaRvrt, WSetMod,
};
use sunspec::{Group, Model};
use time::{Duration, OffsetDateTime};

const LEN: u16 = M::LEN + PfwAbs::LEN + PfwAbsRvrt::LEN + PfwInj::LEN + PfwInjRvrt::LEN;

#[derive(Debug)]
pub(super) struct Control {
    charge: Power,
    discharge: Power,
    profile_at: Option<OffsetDateTime>,
    confirmed: Option<(i32, OffsetDateTime)>,
    release_needed: bool,
    last_confirmed: Option<OffsetDateTime>,
}

impl Control {
    pub(super) fn reset(&mut self) {
        self.profile_at = None;
        self.confirmed = None;
        self.release_needed = false;
    }

    pub(super) fn observe(&mut self, words: &[u16], now: OffsetDateTime) {
        self.profile_at = None;
        let Ok(m) = M::parse(words) else {
            self.confirmed = None;
            return;
        };
        if !eligible(&m) {
            self.confirmed = None;
            return;
        }
        self.profile_at = Some(now);
        self.release_needed = m.w_set_ena == Some(WSetEna::Enabled);
        if !self.release_needed || self.confirmed.is_some_and(|(w, _)| Some(w) != m.w_set) {
            self.confirmed = None;
        }
    }
}

fn eligible(m: &M) -> bool {
    matches!(
        (m.w_set_ena, m.w_set_rvrt_rem),
        (Some(WSetEna::Disabled), Some(0)) | (Some(WSetEna::Enabled), Some(1..=30))
    ) && m.w_set_mod == Some(WSetMod::Watts)
        && m.w_set_sf == Some(0)
        && m.w_set_ena_rvrt == Some(WSetEnaRvrt::Disabled)
        && m.w_set_rvrt_tms == Some(30)
}

impl SunSpec {
    /// Enable the 30-second reversion profile on a commissioned battery.
    #[must_use]
    pub fn with_storage_limits(mut self, charge: Power, discharge: Power) -> Self {
        self.storage = Some(Control {
            charge,
            discharge,
            profile_at: None,
            confirmed: None,
            release_needed: false,
            last_confirmed: None,
        });
        self
    }

    pub(super) fn storage_feedback(&self, now: OffsetDateTime) -> Option<crate::StorageFeedback> {
        let s = self.storage.as_ref()?;
        let fresh = self.link == LinkState::Up
            && s.profile_at
                .is_some_and(|t| t <= now && now - t <= Duration::seconds(5));
        Some(crate::StorageFeedback {
            pending: fresh
                && self.pending.as_ref().is_some_and(|p| {
                    matches!(
                        p.purpose,
                        Purpose::StorageWrite { .. } | Purpose::StorageReadback { .. }
                    )
                }),
            autonomous: fresh && !s.release_needed,
            confirmed: s
                .confirmed
                .filter(|(_, at)| fresh && now - *at < Duration::seconds(30))
                .map(|(w, _)| Power::new(-f64::from(w))),
            last_confirmed: s.last_confirmed,
        })
    }

    pub(super) fn command_storage(
        &mut self,
        command: &Command,
        now: OffsetDateTime,
    ) -> Result<(), DriverError> {
        let state = self.storage.as_mut().expect("commissioned storage");
        if self.link != LinkState::Up
            || self.kind != Some(Kind::Battery)
            || !self.models.has(701)
            || !state
                .profile_at
                .is_some_and(|t| t <= now && now - t <= Duration::seconds(5))
        {
            return Err(DriverError::NotConnected);
        }
        let target = match command {
            Command::OnOff(false) if !state.release_needed => return Ok(()),
            Command::OnOff(false) => None,
            Command::ActivePower(p)
                if p.is_finite()
                    && p.get() <= state.charge.get()
                    && -p.get() <= state.discharge.get()
                    && p.get().abs() <= f64::from(i32::MAX) =>
            {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "bounded above, truncate towards zero within the guard envelope"
                )]
                let watts = -p.get() as i32;
                if state
                    .confirmed
                    .is_some_and(|(w, t)| w == watts && now - t < Duration::seconds(12))
                {
                    return Ok(());
                }
                Some(watts)
            }
            _ => {
                return Err(DriverError::Refused(
                    "target outside commissioned storage envelope".into(),
                ));
            }
        };
        // A newer decision supersedes a write that is still waiting for transport.
        if let Some(p) = &self.pending
            && matches!(p.purpose, Purpose::StorageWrite { .. })
            && self
                .outbox
                .last()
                .is_some_and(|r| r.transaction == p.transaction)
        {
            self.outbox.pop();
            self.pending = None;
        }
        // A subsequent control tick recomputes the target. Never enqueue an old decision behind a poll.
        if self.pending.is_some() {
            return Ok(());
        }
        let body = self
            .models
            .iter()
            .find_map(|(id, base, len)| (id == 704 && len >= LEN).then_some(base))
            .ok_or(DriverError::NotConnected)?;
        let address = body
            .checked_add(M::W_SET_ENA.offset)
            .ok_or(DriverError::NotConnected)?;
        let values = target.map_or_else(
            || vec![0],
            |w| {
                let [a, b, c, d] = w.to_be_bytes();
                vec![1, 1, u16::from_be_bytes([a, b]), u16::from_be_bytes([c, d])]
            },
        );
        state.confirmed = None;
        state.release_needed = true;
        let transaction = self.transaction();
        self.outbox.push(Request {
            transaction,
            unit: self.unit,
            body: RequestBody::Write { address, values },
        });
        self.pending = Some(Pending {
            transaction,
            at: now,
            purpose: Purpose::StorageWrite { address, target },
        });
        Ok(())
    }

    pub(super) fn handle_storage(
        &mut self,
        pending: &Pending,
        response: &Response,
        now: OffsetDateTime,
    ) -> bool {
        let target = match pending.purpose {
            Purpose::StorageWrite { address, target } => {
                let count = if target.is_some() { 4 } else { 1 };
                if response.body == (ResponseBody::WriteAccepted { address, count }) {
                    self.read(
                        address - M::W_SET_ENA.offset,
                        LEN,
                        Purpose::StorageReadback {
                            target,
                            sent: pending.at,
                        },
                        now,
                    );
                    return true;
                }
                target
            }
            Purpose::StorageReadback { target, sent } => {
                if let ResponseBody::Registers(words) = &response.body
                    && response.function == 3
                    && words.len() == usize::from(LEN)
                    && let Ok(m) = M::parse(words)
                    && eligible(&m)
                {
                    let confirmed = match target {
                        Some(w) => {
                            m.w_set_ena == Some(WSetEna::Enabled)
                                && m.w_set == Some(w)
                                && m.w_set_rvrt_rem.is_some_and(|t| t > 0)
                        }
                        None => {
                            m.w_set_ena == Some(WSetEna::Disabled) && m.w_set_rvrt_rem == Some(0)
                        }
                    };
                    if confirmed {
                        let state = self.storage.as_mut().expect("storage readback");
                        state.last_confirmed = Some(now);
                        state.confirmed = target.map(|w| (w, sent));
                        state.release_needed = target.is_some();
                        self.events.push(DriverEvent::Command(CommandOutcome {
                            accepted: true,
                            confirmed: target.map(|w| Power::new(-f64::from(w))),
                            at: now,
                            detail: None,
                        }));
                        return true;
                    }
                }
                target
            }
            _ => return false,
        };
        self.events.push(DriverEvent::Command(CommandOutcome {
            accepted: false,
            confirmed: None,
            at: now,
            detail: Some(format!("battery setting {target:?} was not confirmed")),
        }));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Driver;
    use crate::modbus::Cadence;
    use ems_core::prelude::AssetId;

    fn profile() -> Vec<u16> {
        let mut words = vec![0; usize::from(LEN)];
        words[usize::from(M::W_SET_MOD.offset)] = 1;
        words[usize::from(M::W_SET_RVRT_TMS.offset + 1)] = 30;
        words
    }

    #[test]
    fn storage_requires_matching_unit_echo_readback_and_live_profile() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let power = Power::new(1000.0);
        let mut driver = SunSpec::new(AssetId::new("pack").unwrap(), 3, Cadence::default())
            .with_storage_limits(power, power);
        driver.kind = Some(Kind::Battery);
        driver.link = LinkState::Up;
        driver.models.insert(701, 100, 153);
        driver.models.insert(704, 300, LEN);
        let setpoint = Command::ActivePower(power);
        for point in [
            M::W_SET_SF.offset,
            M::W_SET_RVRT_TMS.offset + 1,
            M::W_SET_MOD.offset,
            M::W_SET_ENA_RVRT.offset,
        ] {
            let mut wrong = profile();
            wrong[usize::from(point)] = 2;
            driver.storage.as_mut().unwrap().observe(&wrong, now);
            assert!(driver.command(&setpoint, now).is_err());
            assert!(driver.poll_transmit().is_none());
        }
        driver.storage.as_mut().unwrap().observe(&profile(), now);
        assert!(
            driver
                .command(&Command::ActivePower(power * 2.0), now)
                .is_err()
        );
        driver.command(&setpoint, now).unwrap();
        assert!(driver.storage_feedback(now).unwrap().pending);
        driver.due = Some(now);
        assert_eq!(driver.poll_deadline(), Some(now + driver.cadence.timeout));
        let pending = driver.pending.clone().unwrap();
        let Purpose::StorageWrite { address, .. } = pending.purpose else {
            panic!("expected storage write")
        };
        let mut reply = Response {
            transaction: pending.transaction,
            unit: 2,
            function: 16,
            body: ResponseBody::WriteAccepted { address, count: 4 },
        };
        driver.handle(&reply, now);
        assert!(driver.pending.is_some());
        reply.unit = 3;
        reply.body = ResponseBody::WriteAccepted { address, count: 3 };
        driver.handle(&reply, now);
        assert!(driver.storage_feedback(now).unwrap().confirmed.is_none());
        assert!(driver.pending.is_none());
        assert!(!driver.storage_feedback(now).unwrap().pending);
        driver.command(&setpoint, now).unwrap();
        reply.transaction = driver.pending.as_ref().unwrap().transaction;
        reply.body = ResponseBody::WriteAccepted { address, count: 4 };
        driver.handle(&reply, now);
        let read = driver.pending.as_ref().unwrap().transaction;
        driver.handle(
            &Response {
                transaction: read,
                unit: 3,
                function: 3,
                body: ResponseBody::Registers(profile()),
            },
            now,
        );
        assert!(
            driver
                .storage_feedback(now)
                .unwrap()
                .last_confirmed
                .is_none()
        );
        driver.on_link(LinkState::Down, now);
        driver.on_link(LinkState::Up, now);
        assert!(driver.command(&setpoint, now).is_err());
        assert!(driver.storage_feedback(now).unwrap().confirmed.is_none());
    }
}
