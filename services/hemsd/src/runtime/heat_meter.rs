//! Local, read-only heat-meter observations on a dedicated wired M-Bus converter.
use std::{io, time::Duration};

use ems_drv::mbus::{HeatMeter, MAX_FRAME, ReadError};
use ems_service::Shutdown;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
    time::{Instant, sleep, timeout},
};
use tokio_serial::{ClearBuffer, SerialPort, SerialStream};

use crate::config::{HeatMeterSettings, SerialBus, SerialParity};

/// Upper bound including the reset, ACK and a 261-byte reply at 2400 baud.
const READ_TIMEOUT: Duration = Duration::from_secs(3);

/// Result of the most recent readout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// First poll in progress.
    Waiting,
    /// Current qualified register available.
    Live,
    /// Converter missing, timed out, or observation expired.
    Unavailable,
    /// A different meter answered.
    WrongMeter,
    /// The wire data or heat register could not be qualified.
    InvalidReply,
}

impl State {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Waiting => "awaiting first reading",
            Self::Live => "live",
            Self::Unavailable => "unavailable · check converter and wiring",
            Self::WrongMeter => "unexpected meter · check address, manufacturer and ID",
            Self::InvalidReply => {
                "invalid reply · check meter status and supported register profile"
            }
        }
    }
}

/// Installer diagnostics. The decimal counter is carried as a string on the wire.
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Reading {
    /// Commissioned M-Bus primary address.
    pub address: u8,
    /// Commissioned FLAG code.
    pub manufacturer: String,
    /// Commissioned eight-digit meter ID.
    pub meter_id: String,
    /// Readout status.
    pub state: State,
    /// Current cumulative delivered thermal energy in exact Wh.
    pub delivered_heat_wh: Option<String>,
    /// Gateway receipt time of the last successful read in Unix seconds.
    pub last_read_at: Option<i64>,
}

/// Handle to the latest reading.
#[derive(Clone)]
pub struct Observer(watch::Receiver<(Reading, Instant)>);

impl Observer {
    /// Start polling a dedicated local converter.
    ///
    /// # Errors
    /// Returns an error for an invalid converter, polling interval or meter identity.
    pub fn start(point: &HeatMeterSettings, shutdown: Shutdown) -> anyhow::Result<Self> {
        let meter = point.binding().map_err(anyhow::Error::msg)?;
        let period = Duration::from_secs(point.poll_s);
        let initial = Reading {
            address: point.address,
            manufacturer: point.manufacturer.clone(),
            meter_id: point.meter_id.clone(),
            state: State::Waiting,
            delivered_heat_wh: None,
            last_read_at: None,
        };
        let (tx, rx) = watch::channel((initial, Instant::now()));
        let bus = SerialBus {
            path: point.path.clone(),
            baud: point.baud,
            parity: SerialParity::Even,
            stop_bits: 1,
        };
        tokio::spawn(ems_service::shutdown::until(shutdown, async move {
            let mut port = None;
            loop {
                let result = timeout(READ_TIMEOUT, async {
                    if port.is_none() {
                        port = Some(super::serial::open(&bus).map_err(|_| State::Unavailable)?);
                    }
                    let tty = port.as_mut().ok_or(State::Unavailable)?;
                    tty.clear(ClearBuffer::Input)
                        .map_err(|_| State::Unavailable)?;
                    read(tty, meter).await
                })
                .await
                .unwrap_or(Err(State::Unavailable));
                if result.is_err() {
                    port = None;
                }
                tx.send_modify(|(reading, expiry)| match result {
                    Ok(wh) => {
                        reading.state = State::Live;
                        reading.delivered_heat_wh = Some(wh);
                        reading.last_read_at =
                            Some(time::OffsetDateTime::now_utc().unix_timestamp());
                        *expiry = Instant::now() + period + READ_TIMEOUT;
                    }
                    Err(state) => {
                        reading.state = state;
                        reading.delivered_heat_wh = None;
                    }
                });
                sleep(period).await;
            }
        }));
        Ok(Self(rx))
    }

    /// Return only values qualified within the configured freshness window.
    #[must_use]
    pub fn reading(&self) -> Reading {
        let (mut reading, expiry) = self.0.borrow().clone();
        if reading.state == State::Live
            && (Instant::now() >= expiry || self.0.has_changed().is_err())
        {
            reading.state = State::Unavailable;
            reading.delivered_heat_wh = None;
        }
        reading
    }
}

async fn first_byte(tty: &mut SerialStream, request: &[u8; 5]) -> Result<u8, State> {
    async fn exchange(tty: &mut SerialStream, request: &[u8; 5]) -> io::Result<u8> {
        tty.write_all(request).await?;
        let first = tty.read_u8().await?;
        if first != request[0] {
            return Ok(first);
        }
        // Some level converters echo the exact request before the response.
        let mut tail = [0; 4];
        tty.read_exact(&mut tail).await?;
        if tail != request[1..] {
            return Err(io::ErrorKind::InvalidData.into());
        }
        tty.read_u8().await
    }
    exchange(tty, request).await.map_err(|_| State::Unavailable)
}

async fn read(tty: &mut SerialStream, meter: HeatMeter) -> Result<String, State> {
    let [reset, readout] = meter.requests();
    if first_byte(tty, &reset).await? != 0xe5 || first_byte(tty, &readout).await? != 0x68 {
        return Err(State::InvalidReply);
    }
    let mut frame = [0; MAX_FRAME];
    frame[0] = 0x68;
    frame[1] = tty.read_u8().await.map_err(|_| State::Unavailable)?;
    let total = usize::from(frame[1]) + 6;
    tty.read_exact(&mut frame[2..total])
        .await
        .map_err(|_| State::Unavailable)?;
    // A second reply in the same burst (e.g. colliding addresses) disqualifies the read.
    // 20 ms covers more than four 8E1 characters at the lowest supported baud.
    match timeout(Duration::from_millis(20), tty.read_u8()).await {
        Ok(Ok(_)) => return Err(State::InvalidReply),
        Ok(Err(_)) => return Err(State::Unavailable),
        Err(_) => {}
    }
    meter
        .delivered_wh(&frame[..total])
        .map(|wh| wh.to_string())
        .map_err(|error| match error {
            ReadError::Identity => State::WrongMeter,
            ReadError::Frame | ReadError::Register => State::InvalidReply,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stalled_or_stopped_observer_withholds_the_last_total() {
        // SR-169. The API's freshness bound also holds if the worker stops scheduling polls.
        let live = Reading {
            address: 17,
            manufacturer: "KAM".into(),
            meter_id: "06855817".into(),
            state: State::Live,
            delivered_heat_wh: Some("0".into()),
            last_read_at: Some(123),
        };
        let (sender, receiver) = watch::channel((live, Instant::now() + READ_TIMEOUT));
        let observer = Observer(receiver);
        assert_eq!(observer.reading().delivered_heat_wh.as_deref(), Some("0"));
        sender.send_modify(|(_, expiry)| *expiry = Instant::now());
        let stale = observer.reading();
        assert_eq!(stale.state, State::Unavailable);
        assert_eq!(stale.delivered_heat_wh, None);
        assert_eq!(stale.last_read_at, Some(123));
        sender.send_modify(|(_, expiry)| *expiry = Instant::now() + READ_TIMEOUT);
        drop(sender);
        assert_eq!(observer.reading().state, State::Unavailable);
    }
}
