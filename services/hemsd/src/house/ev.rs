//! A fixed three-phase wallbox. Its outlet register counts energy before charging losses.
use anyhow::{Context, Result, ensure};
use ems_core::prelude::{Current, Energy, Power};
use ems_sim::{EvseSim, VehicleSim};
use time::{Duration, OffsetDateTime};
use tokio_modbus::{ExceptionCode, Request, Response};

use crate::scenario::VEHICLE_CAPACITY;

#[derive(Debug)]
pub(super) struct Wallbox {
    outlet: EvseSim,
    arrival: OffsetDateTime,
    departure: OffsetDateTime,
    plugged: bool,
    amps: u16,
    power: Power,
    delivered: Energy,
}

impl Wallbox {
    pub(super) fn new(scenario: &crate::Scenario) -> Result<Self> {
        let session = scenario.ev.context("the reference day has no EV session")?;
        let rating = scenario.config.evse.context("missing wallbox rating")?;
        ensure!(
            (6.0..=32.0).contains(&rating.max_current.get()),
            "wallbox fixture requires 6–32 A"
        );
        let mut outlet = EvseSim::new().with_vehicle(VehicleSim {
            capacity: VEHICLE_CAPACITY,
            stored: session.energy_now,
            max_charge: Power::from_kw(11.0),
            efficiency: 0.92,
            efficiency_single_phase: crate::scenario::SINGLE_PHASE_EFFICIENCY,
        });
        outlet.max_current = rating.max_current;
        Ok(Self {
            outlet,
            arrival: scenario.start() + session.arrival,
            departure: scenario.start() + session.departure,
            plugged: false,
            amps: 0,
            power: Power::ZERO,
            delivered: Energy::ZERO,
        })
    }

    pub(super) fn step(&mut self, at: OffsetDateTime, dt: Duration) -> Power {
        if dt == Duration::ZERO {
            self.plugged = (self.arrival..self.departure).contains(&at);
            return self.power;
        }
        let active = ((at + dt).min(self.departure) - at.max(self.arrival)).max(Duration::ZERO);
        let desired =
            Power::new(f64::from(self.amps) * 3.0 * ems_core::units::NOMINAL_VOLTAGE.get());
        let actual = self.outlet.step(desired, active);
        self.delivered += actual.over(active);
        self.plugged = (self.arrival..self.departure).contains(&(at + dt));
        self.power = if self.plugged {
            actual
        } else {
            self.amps = 0;
            Power::ZERO
        };
        actual * (active / dt)
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(super) fn request(&mut self, request: &Request<'_>) -> Result<Response, ExceptionCode> {
        let vehicle = self
            .outlet
            .vehicle
            .as_ref()
            .ok_or(ExceptionCode::ServerDeviceFailure)?;
        let [hi, lo] = super::appliance_map::watt_hours(self.delivered);
        let words = [
            u16::from(self.plugged),
            self.power.get().round() as u16,
            self.amps,
            (10_000.0 * vehicle.stored.get() / vehicle.capacity.get()).round() as u16,
            vehicle.capacity.get() as u16,
            hi,
            lo,
        ];
        let (response, amps) = super::appliance_map::request(request, words, 0..=32)?;
        if let Some(amps) = amps {
            let current = Current::new(f64::from(amps));
            if amps != 0
                && (!self.plugged
                    || current < self.outlet.min_current
                    || current > self.outlet.max_current)
            {
                return Err(ExceptionCode::IllegalDataValue);
            }
            self.amps = amps;
        }
        Ok(response)
    }
}

pub(super) fn write_map(
    out: &mut String,
    ev: &ems_core::prelude::AssetId,
    modbus: std::net::SocketAddr,
) -> std::fmt::Result {
    use super::UNIT_EV;
    use std::fmt::Write as _;
    writeln!(
        out,
        r#"
[[drivers]]
kind = "registers"
asset = "{ev}"
address = "{modbus}"
unit = {UNIT_EV}
points = [
  {{ field = "power", register = 1 }},
  {{ field = "soc", register = 3, scale = 0.0001 }},
  {{ field = "vehicle_capacity_wh", register = 4 }},
  {{ field = "energy_in", register = 5, word = "u32" }},
  {{ field = "vehicle_connected", register = 0 }}
]
writes = [{{ register = 2, command = "charging_amps" }}]"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_modbus::Response::ReadHoldingRegisters;

    #[test]
    fn a_departure_closes_the_outlet_and_refused_writes_preserve_its_readings() {
        let scenario = crate::Scenario::winter_with_grid_event(crate::HouseholdConfig::default());
        let mut ev = Wallbox::new(&scenario).unwrap();
        let departure = ev.departure;
        ev.step(departure - Duration::MINUTE, Duration::ZERO);
        ev.request(&Request::WriteSingleRegister(2, 10)).unwrap();
        let read = Request::ReadHoldingRegisters(0, 7);
        let before = ev.request(&read).unwrap();
        for write in [
            Request::WriteSingleRegister(0, 1),
            Request::WriteSingleRegister(2, 5),
            Request::WriteSingleRegister(2, 32),
            Request::WriteMultipleRegisters(2, vec![10, 0].into()),
        ] {
            assert!(ev.request(&write).is_err());
            assert_eq!(ev.request(&read).unwrap(), before);
        }
        assert_eq!(
            ev.step(departure - Duration::MINUTE, Duration::minutes(2))
                .get(),
            3450.0
        );
        let ReadHoldingRegisters(words) = ev.request(&read).unwrap() else {
            panic!("missing registers")
        };
        assert_eq!(&words[0..3], &[0, 0, 0]);
        assert_eq!(&words[5..7], &[0, 115]);
        assert!(ev.request(&Request::WriteSingleRegister(2, 6)).is_err());
        assert_eq!(ev.step(departure, Duration::MINUTE), Power::ZERO);
    }
}
