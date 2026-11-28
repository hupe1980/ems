//! A read-only gateway observes the winter house at each appliance control boundary.
use super::{Physics, UNIT_METER, modbus};
use anyhow::{Result, ensure};
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
    pub(super) boundaries: Vec<Measurement>,
}

impl Drop for Grid {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Grid {
    pub(super) async fn start(at: OffsetDateTime) -> Result<Self> {
        let config = crate::HouseholdConfig {
            battery: None,
            ..Default::default()
        };
        Self::configured(at, &crate::Scenario::winter_with_grid_event(config)).await
    }

    pub(super) async fn configured(at: OffsetDateTime, scenario: &crate::Scenario) -> Result<Self> {
        let mut plant = Physics::new(scenario)?;
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
            boundaries: Vec::new(),
        })
    }

    pub(super) async fn sample(&mut self) -> Result<Measurement> {
        let sample = Self::read(&mut self.driver, &mut self.socket, self.at).await?;
        if self.at.unix_timestamp() % 900 == 0
            && self
                .boundaries
                .last()
                .is_none_or(|last| last.at != sample.at)
        {
            self.boundaries.push(sample);
        }
        Ok(sample)
    }

    pub(super) async fn battery_soc(&mut self) -> Result<f64> {
        let mut reader = SunSpec::new(
            AssetId::new("battery")?,
            super::UNIT_BATTERY,
            Cadence::default(),
        )
        .listening_only();
        reader.on_link(LinkState::Up, self.at);
        let sample = Self::read(&mut reader, &mut self.socket, self.at).await?;
        Ok(sample
            .soc
            .ok_or_else(|| anyhow::anyhow!("missing battery state of charge"))?
            .fraction())
    }

    async fn read(
        driver: &mut SunSpec,
        socket: &mut TcpStream,
        at: OffsetDateTime,
    ) -> Result<Measurement> {
        driver.on_timeout(at);
        while let Some(request) = driver.poll_transmit() {
            socket.write_all(&request).await?;
            let mut frame = [0_u8; 260];
            socket.read_exact(&mut frame[..6]).await?;
            let len = usize::from(u16::from_be_bytes([frame[4], frame[5]]));
            ensure!((2..=254).contains(&len), "invalid Modbus reply length");
            socket.read_exact(&mut frame[6..6 + len]).await?;
            driver.on_bytes(&frame[..6 + len], at)?;
        }
        while let Some(event) = driver.poll_event() {
            if let DriverEvent::Measured(sample) = event {
                ensure!(
                    sample.at == at && sample.power.is_some(),
                    "missing fresh device power"
                );
                return Ok(sample);
            }
        }
        anyhow::bail!("device did not report")
    }

    pub(super) async fn quarter(&mut self, applied_w: i32) -> Result<Measurement> {
        self.advance(Power::new(f64::from(applied_w)), 900)?;
        self.sample().await
    }

    pub(super) fn advance(&mut self, applied: Power, seconds: u16) -> Result<()> {
        let mut plant = self
            .plant
            .lock()
            .map_err(|_| anyhow::anyhow!("winter house unavailable"))?;
        for _ in 0..seconds {
            plant.step_with_load(self.at, Duration::SECOND, applied);
            self.at += Duration::SECOND;
        }
        plant.step(self.at, Duration::ZERO);
        Ok(())
    }
}
