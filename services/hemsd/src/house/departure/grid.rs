//! A read-only gateway observes the winter house at each OCPP interval boundary.
use super::super::{Physics, UNIT_METER, modbus};
use anyhow::{Context, Result, ensure};
use ems_core::prelude::{AssetId, Measurement, Power};
use ems_drv::{
    Driver, DriverEvent, LinkState,
    modbus::{Cadence, SunSpec},
};
use std::sync::{Arc, Mutex};
use time::{Duration, OffsetDateTime};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

pub(super) struct Grid {
    plant: Arc<Mutex<Physics>>,
    server: JoinHandle<()>,
    socket: TcpStream,
    driver: SunSpec,
    at: OffsetDateTime,
}

impl Drop for Grid {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Grid {
    pub(super) async fn start(at: OffsetDateTime) -> Result<Self> {
        let scenario = crate::Scenario::winter_with_grid_event(crate::HouseholdConfig {
            battery: None,
            ..Default::default()
        });
        let mut plant = Physics::new(&scenario)?;
        plant.step(at, Duration::ZERO);
        let plant = Arc::new(Mutex::new(plant));
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let socket = TcpStream::connect(listener.local_addr()?).await?;
        socket.set_nodelay(true)?;
        let mut driver = SunSpec::new(AssetId::new("connection")?, UNIT_METER, Cadence::default())
            .listening_only();
        driver.on_link(LinkState::Up, at);
        let server = tokio::spawn(modbus::serve(listener, Arc::clone(&plant)));
        Ok(Self {
            plant,
            server,
            socket,
            driver,
            at,
        })
    }

    pub(super) async fn sample(&mut self) -> Result<Measurement> {
        self.driver.on_timeout(self.at);
        while let Some(request) = self.driver.poll_transmit() {
            self.socket.write_all(&request).await?;
            let mut frame = [0_u8; 260];
            self.socket.read_exact(&mut frame[..6]).await?;
            let len = usize::from(u16::from_be_bytes([frame[4], frame[5]]));
            ensure!((2..=254).contains(&len), "invalid Modbus reply length");
            self.socket.read_exact(&mut frame[6..6 + len]).await?;
            self.driver.on_bytes(&frame[..6 + len], self.at)?;
        }
        while let Some(event) = self.driver.poll_event() {
            if let DriverEvent::Measured(sample) = event {
                ensure!(
                    sample.at == self.at && sample.power.is_some(),
                    "missing fresh connection power"
                );
                return Ok(sample);
            }
        }
        anyhow::bail!("connection meter did not report")
    }

    pub(super) async fn quarter(&mut self, applied_w: i32) -> Result<Measurement> {
        {
            let mut plant = self
                .plant
                .lock()
                .map_err(|_| anyhow::anyhow!("winter house unavailable"))?;
            for _ in 0..900 {
                plant.step_with_ev(self.at, Duration::SECOND, Power::new(f64::from(applied_w)));
                self.at += Duration::SECOND;
            }
            // The transaction profile has expired at this boundary.
            plant.step(self.at, Duration::ZERO);
        }
        self.sample().await
    }
}

#[derive(Debug)]
pub(super) struct Balance {
    pub import: u32,
    pub export: u32,
    pub interruption: u32,
}

impl Balance {
    pub(super) fn from_samples(samples: [&Measurement; 4]) -> Result<Self> {
        use rust_decimal::prelude::ToPrimitive;
        let counters = |sample: &Measurement| -> Result<(u32, u32)> {
            let wh = |energy: Option<ems_core::prelude::Energy>| {
                energy
                    .map(ems_core::prelude::Energy::get)
                    .filter(|wh| wh.fract() == 0.0)
                    .and_then(|wh| wh.to_u32())
                    .context("missing whole-Wh connection register")
            };
            Ok((wh(sample.energy_in)?, wh(sample.energy_out)?))
        };
        let [open, pause, resume, close] = samples.map(counters);
        let points = [open?, pause?, resume?, close?];
        for (pair, registers) in samples.windows(2).zip(points.windows(2)) {
            ensure!(
                pair[1].at - pair[0].at == Duration::minutes(15),
                "connection readings must span consecutive quarters"
            );
            let (before, after) = (registers[0], registers[1]);
            ensure!(
                after.0 >= before.0 && after.1 >= before.1,
                "connection register moved backwards"
            );
        }
        let [open, pause, resume, close] = points;
        Ok(Self {
            import: close.0 - open.0,
            export: close.1 - open.1,
            interruption: resume.0 - pause.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ems_core::prelude::Energy;

    #[test]
    fn only_complete_consecutive_directional_registers_close_the_window() {
        let samples = [0_i32, 1, 2, 3].map(|i| {
            let mut sample = Measurement::power(
                OffsetDateTime::UNIX_EPOCH + Duration::minutes(i64::from(i) * 15),
                Power::ZERO,
            );
            sample.energy_in = Some(Energy::new(100.0 + 5.0 * f64::from(i)));
            sample.energy_out = Some(Energy::new(50.0 + f64::from(i)));
            sample
        });
        let parse = |s: &[Measurement; 4]| Balance::from_samples(s.each_ref());
        let balance = parse(&samples).unwrap();
        assert_eq!(
            (balance.import, balance.export, balance.interruption),
            (15, 3, 5)
        );

        let mut broken = samples;
        broken[1].energy_in = None;
        assert!(parse(&broken).is_err());
        broken = samples;
        broken[2].energy_out = Some(Energy::ZERO);
        assert!(parse(&broken).is_err());
        broken = samples;
        broken[1].at = broken[0].at;
        assert!(parse(&broken).is_err());
    }
}
