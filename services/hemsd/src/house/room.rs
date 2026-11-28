//! A heating-only appliance with three SG Ready states on a declared Modbus map.
use super::registers::signed;
use anyhow::{Result, ensure};
use ems_core::prelude::Power;
use ems_device::SgReadyState;
use ems_sim::BuildingSim;
use time::Duration;
use tokio_modbus::{ExceptionCode, Request, Response};

#[derive(Debug)]
pub(super) struct Room {
    building: BuildingSim,
    mode: SgReadyState,
    normal_target_c: f64,
    measured: Power,
}

impl Room {
    pub(super) fn new(config: crate::site::HeatPumpConfig) -> Result<Self> {
        ensure!(
            config.power > Power::ZERO
                && config.power.get() <= f64::from(i16::MAX)
                && config.comfort_min_c < config.comfort_max_c,
            "invalid room heating fixture"
        );
        let normal_target_c = (config.comfort_min_c + 0.5).min(config.comfort_max_c);
        let building = BuildingSim {
            nominal_electrical: config.power,
            min_electrical: config.power * 0.3,
            thermostat_set_c: normal_target_c,
            thermostat_max_c: config.comfort_max_c,
            ..BuildingSim::new(crate::scenario::OPENING_ROOM_C)
        };
        Ok(Self {
            building,
            mode: SgReadyState::Normal,
            normal_target_c,
            measured: Power::ZERO,
        })
    }

    pub(super) fn step(&mut self, dt: Duration, outdoor: f64, window: f64) -> Power {
        if dt > Duration::ZERO {
            self.building.thermostat_set_c = if self.mode == SgReadyState::Boost {
                self.building.thermostat_max_c
            } else {
                self.normal_target_c
            };
            let ceiling = if self.mode == SgReadyState::Limited {
                Power::ZERO
            } else {
                self.building.nominal_electrical
            };
            self.measured = self.building.step(ceiling, outdoor, window, dt);
        }
        self.measured
    }

    pub(super) fn request(&mut self, request: &Request<'_>) -> Result<Response, ExceptionCode> {
        let words = [
            signed(self.building.indoor_c() * 10.0),
            signed(self.measured.get()),
            u16::from(self.mode.number()),
        ];
        let (response, mode) = super::appliance_map::request(request, words, 1..=3)?;
        if let Some(mode) = mode {
            self.mode = u8::try_from(mode)
                .ok()
                .and_then(SgReadyState::from_number)
                .ok_or(ExceptionCode::IllegalDataValue)?;
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limited_room_cools_and_rejects_unknown_contact_states() {
        let mut room = Room::new(crate::HouseholdConfig::default().heat_pump.unwrap()).unwrap();
        room.request(&Request::WriteSingleRegister(2, 1)).unwrap();
        let initial = room.building.indoor_c();
        for _ in 0..360 {
            assert_eq!(room.step(Duration::MINUTE, -3.0, 0.0), Power::ZERO);
        }
        assert!(room.building.indoor_c() < initial);
        let read = Request::ReadHoldingRegisters(0, 3);
        let before = room.request(&read).unwrap();
        for write in [
            Request::WriteSingleRegister(2, 4),
            Request::WriteSingleRegister(2, 257),
            Request::WriteMultipleRegisters(1, vec![0, 3].into()),
        ] {
            assert!(room.request(&write).is_err());
            assert_eq!(room.request(&read).unwrap(), before);
        }
        let cold = room.building.indoor_c();
        room.request(&Request::WriteSingleRegister(2, 3)).unwrap();
        assert!(room.step(Duration::MINUTE, -3.0, 0.0) > Power::ZERO);
        assert!(room.building.indoor_c() > cold);
    }
}
