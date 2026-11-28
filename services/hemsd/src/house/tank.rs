//! A relay-driven cylinder on the simulator's declared Modbus map.
use super::registers::signed;
use anyhow::{Context, Result};
use ems_core::prelude::{Asset, Energy, Power};
use ems_sim::TankSim;
use time::Duration;
use tokio_modbus::{ExceptionCode, Request, Response};

#[derive(Debug)]
pub(super) struct Tank {
    plant: TankSim,
    minimum_c: f64,
    capacity_per_k: f64,
    enabled: bool,
    unmet: Energy,
    measured: Power,
}

impl Tank {
    pub(super) fn new(config: &crate::HouseholdConfig) -> Result<Self> {
        let home = crate::Household::build(config)?;
        let tank = home
            .site
            .assets
            .iter()
            .find_map(|asset| match asset {
                Asset::Dhw(tank) => Some(tank),
                _ => None,
            })
            .context("the house has no hot-water tank")?;
        let plant = TankSim {
            cop: tank.cop,
            standing_loss: tank.standing_loss,
            ..TankSim::new(tank.usable_heat(), tank.heater)
        };
        anyhow::ensure!(
            tank.heat_capacity_kwh_per_k() > 0.0 && tank.heater.get() <= f64::from(i16::MAX),
            "tank outside simulator register range"
        );
        Ok(Self {
            plant,
            minimum_c: tank.t_min_c,
            capacity_per_k: tank.heat_capacity_kwh_per_k(),
            enabled: false,
            unmet: Energy::ZERO,
            measured: Power::ZERO,
        })
    }

    pub(super) fn step(&mut self, dt: Duration, draw: Energy) -> Power {
        if dt > Duration::ZERO {
            let wanted = if self.enabled {
                self.plant.heater
            } else {
                Power::ZERO
            };
            let (power, short) = self.plant.step(wanted, draw, dt);
            self.measured = power;
            self.unmet += short;
        }
        self.measured
    }

    pub(super) fn request(&mut self, request: &Request<'_>) -> Result<Response, ExceptionCode> {
        let temperature = self.minimum_c + self.plant.stored.kwh() / self.capacity_per_k;
        let [stored_hi, stored_lo] = super::appliance_map::watt_hours(self.plant.stored);
        let [unmet_hi, unmet_lo] = super::appliance_map::watt_hours(self.unmet);
        let words = [
            signed(temperature * 10.0),
            signed(self.measured.get()),
            u16::from(self.enabled),
            stored_hi,
            stored_lo,
            unmet_hi,
            unmet_lo,
        ];
        let (response, setting) = super::appliance_map::request(request, words, 0..=1)?;
        if let Some(setting) = setting {
            self.enabled = setting == 1;
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refused_writes_preserve_the_relay_and_sensor_registers() {
        let mut tank = Tank::new(&crate::HouseholdConfig::default()).unwrap();
        tank.request(&Request::WriteSingleRegister(2, 1)).unwrap();
        let read = Request::ReadHoldingRegisters(0, 7);
        let before = tank.request(&read).unwrap();
        for request in [
            Request::ReadHoldingRegisters(0, 0),
            Request::ReadHoldingRegisters(0, 126),
            Request::WriteMultipleRegisters(2, vec![].into()),
            Request::WriteMultipleRegisters(2, vec![0; 124].into()),
        ] {
            assert_eq!(tank.request(&request), Err(ExceptionCode::IllegalDataValue));
        }
        for request in [
            Request::WriteSingleRegister(2, 7),
            Request::WriteSingleRegister(0, 1),
            Request::WriteMultipleRegisters(1, vec![0, 0].into()),
            Request::WriteMultipleRegisters(2, vec![0, 0].into()),
            Request::ReadHoldingRegisters(6, 2),
        ] {
            assert!(tank.request(&request).is_err());
            assert_eq!(tank.request(&read).unwrap(), before);
        }
        tank.plant.stored = Energy::ZERO;
        tank.request(&Request::WriteSingleRegister(2, 0)).unwrap();
        tank.step(Duration::HOUR, Energy::ZERO);
        tank.step(Duration::HOUR, Energy::new(250.0));
        assert_eq!(
            tank.request(&Request::ReadHoldingRegisters(3, 4)).unwrap(),
            Response::ReadHoldingRegisters(vec![0, 0, 0, 250])
        );
    }
}
