//! Virtual model 704 participant. The four-register watt-mode request is atomic.
use ems_core::prelude::Power;
use sunspec::Group;
use sunspec::models::model704::{Model704 as M, PfwAbs, PfwAbsRvrt, PfwInj, PfwInjRvrt};
use time::Duration;

use super::registers::Refusal;

pub(super) const LEN: u16 = M::LEN + PfwAbs::LEN + PfwAbsRvrt::LEN + PfwInj::LEN + PfwInjRvrt::LEN;
const REVERSION_SECONDS: u16 = 30;

#[derive(Debug, Clone)]
pub(super) struct Storage {
    rating: Power,
    pub(super) remaining: Duration,
    pub(super) target: Option<Power>,
}

impl Storage {
    pub(super) fn new(rating: Power, body: &mut [u16]) -> Self {
        // Unsupported optional enum, unsigned and signed points carry their SunSpec sentinels.
        body.fill(u16::MAX);
        for p in [
            M::W_SET_PCT,
            M::W_SET_PCT_RVRT,
            M::VAR_SET_PCT,
            M::VAR_SET_PCT_RVRT,
            M::PF_SF,
            M::W_MAX_LIM_PCT_SF,
            M::W_SET_PCT_SF,
            M::VAR_SET_SF,
            M::VAR_SET_PCT_SF,
        ] {
            body[usize::from(p.offset)] = i16::MIN.cast_unsigned();
        }
        for p in [M::VAR_SET, M::VAR_SET_RVRT] {
            let at = usize::from(p.offset);
            body[at..at + 2].copy_from_slice(&[0x8000, 0]);
        }
        body[usize::from(M::W_SET_ENA.offset)..usize::from(M::W_SET_PCT.offset)].fill(0);
        body[usize::from(M::W_SET_MOD.offset)] = 1;
        body[usize::from(M::W_SET_SF.offset)] = 0;
        body[usize::from(M::W_SET_ENA_RVRT.offset)..=usize::from(M::W_SET_RVRT_REM.offset + 1)]
            .fill(0);
        body[usize::from(M::W_SET_RVRT_TMS.offset + 1)] = REVERSION_SECONDS;
        Self {
            rating,
            remaining: Duration::ZERO,
            target: None,
        }
    }

    pub(super) fn write(
        &mut self,
        body: &mut [u16],
        offset: usize,
        values: &[u16],
    ) -> Result<(), Refusal> {
        let enable = usize::from(M::W_SET_ENA.offset);
        if offset != enable {
            return Err(Refusal::Address);
        }
        match values {
            [0] => self.release(body),
            [1, 1, high, low] => {
                let watts = (u32::from(*high) << 16 | u32::from(*low)).cast_signed();
                if watts == i32::MIN || f64::from(watts).abs() > self.rating.get() {
                    return Err(Refusal::Value);
                }
                body[enable..enable + 4].copy_from_slice(values);
                self.target = Some(Power::new(-f64::from(watts)));
                self.remaining = Duration::seconds(i64::from(REVERSION_SECONDS));
                self.update_timer(body);
            }
            _ => return Err(Refusal::Value),
        }
        Ok(())
    }

    fn release(&mut self, body: &mut [u16]) {
        body[usize::from(M::W_SET_ENA.offset)] = 0;
        let at = usize::from(M::W_SET.offset);
        body[at..at + 2].fill(0);
        self.target = None;
        self.remaining = Duration::ZERO;
        self.update_timer(body);
    }

    fn update_timer(&self, body: &mut [u16]) {
        let seconds =
            self.remaining.whole_seconds() + i64::from(self.remaining.subsec_nanoseconds() > 0);
        body[usize::from(M::W_SET_RVRT_REM.offset + 1)] =
            u16::try_from(seconds).expect("at most 30 seconds");
    }

    pub(super) fn advance(&mut self, body: &mut [u16], elapsed: Duration) {
        if elapsed >= self.remaining {
            self.release(body);
        } else {
            self.remaining -= elapsed;
            self.update_timer(body);
        }
    }
}
