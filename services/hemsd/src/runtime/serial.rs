//! One RTU master per serial port. Each address keeps its own discovery and freshness state.

use std::{io, time::Duration};

use ems_drv::{
    LinkState,
    modbus::{
        frame,
        probe::{Identity, Probe},
        rtu::ReadRequest,
    },
};
use ems_service::Shutdown;
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{Instant, sleep, timeout};
use tokio_serial::{DataBits, FlowControl, Parity, SerialStream, StopBits};

use super::transport::Shared;
use crate::config::{SerialBus, SerialParity};
use crate::drivers::Attached;

pub(super) struct Meter {
    pub on: Attached,
    pub unit: u8,
    pub timeout: Duration,
}

/// Reopen a lost port. A missing unit stays local to its own slot on the bus.
pub(super) async fn run(bus: SerialBus, meters: Vec<Meter>, registry: Shared, shutdown: Shutdown) {
    if meters.is_empty() {
        return;
    }
    let mut backoff = Duration::from_secs(1);
    loop {
        if shutdown.is_triggered() {
            return;
        }
        match open(&bus) {
            Ok(mut port) => {
                backoff = Duration::from_secs(1);
                links(&meters, &registry, LinkState::Connecting).await;
                tokio::select! {
                    biased;
                    () = shutdown.clone().wait() => return,
                    error = poll(&mut port, &meters, &registry, Timing::for_bus(&bus)) => {
                        tracing::warn!(path = %bus.path.display(), %error, "serial bus disconnected");
                    }
                }
                links(&meters, &registry, LinkState::Down).await;
            }
            Err(error) => {
                tracing::warn!(path = %bus.path.display(), %error, "cannot open serial bus");
            }
        }
        tokio::select! {
            () = shutdown.clone().wait() => return,
            () = sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

pub(super) fn open(bus: &SerialBus) -> io::Result<SerialStream> {
    let path = bus
        .path
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid serial port path"))?;
    let parity = match bus.parity {
        SerialParity::None => Parity::None,
        SerialParity::Even => Parity::Even,
        SerialParity::Odd => Parity::Odd,
    };
    #[cfg(target_os = "macos")]
    let baud = {
        // Darwin PTYs reject the speed ioctl. They have no physical clock, but the
        // RTU deadlines still use the configured baud. USB UARTs use their real rate.
        let device = std::fs::canonicalize(path)?;
        let is_pty = device
            .to_str()
            .and_then(|s| s.strip_prefix("/dev/ttys"))
            .is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()));
        if is_pty { 0 } else { bus.baud }
    };
    #[cfg(not(target_os = "macos"))]
    let baud = bus.baud;
    let builder = tokio_serial::new(path, baud)
        .data_bits(DataBits::Eight)
        .flow_control(FlowControl::None)
        .stop_bits(if bus.stop_bits == 2 {
            StopBits::Two
        } else {
            StopBits::One
        })
        .parity(parity);
    // The serialport builder acquires exclusive access when opening the TTY.
    Ok(SerialStream::open(&builder)?)
}

#[derive(Clone, Copy)]
struct Timing {
    character_gap: Duration,
    frame_gap: Duration,
}

impl Timing {
    fn for_bus(bus: &SerialBus) -> Self {
        // Modbus Serial Line V1.02 §2.5.1.1 specifies fixed intervals above 19,200 baud.
        let bits =
            9 + u64::from(bus.stop_bits) + u64::from(!matches!(bus.parity, SerialParity::None));
        let interval = |half_chars: u64, fast_micros: u64| {
            let micros = if bus.baud > 19_200 {
                fast_micros
            } else {
                (bits * half_chars * 500_000_u64).div_ceil(u64::from(bus.baud))
            };
            Duration::from_micros(micros)
        };
        Self {
            character_gap: interval(3, 750),
            frame_gap: interval(7, 1_750),
        }
    }
}

async fn links(meters: &[Meter], registry: &Shared, state: LinkState) {
    let mut guard = registry.lock().await;
    let now = OffsetDateTime::now_utc();
    for meter in meters {
        guard.on_link(meter.on.driver, state, now);
    }
}

async fn poll(
    port: &mut SerialStream,
    meters: &[Meter],
    registry: &Shared,
    timing: Timing,
) -> io::Error {
    let mut retry_at = vec![Instant::now(); meters.len()];
    loop {
        let mut next = Instant::now() + Duration::from_secs(30);
        for (meter, retry) in meters.iter().zip(&mut retry_at) {
            if *retry > Instant::now() {
                next = next.min(*retry);
                continue;
            }
            let id = meter.on.driver;
            let bytes = {
                let mut guard = registry.lock().await;
                let now = OffsetDateTime::now_utc();
                // A request queued by the previous response has not reached the wire yet.
                let mut bytes = guard.poll_transmit_of(id);
                if bytes.is_none() {
                    if guard.deadline_of(id).is_none_or(|at| at <= now) {
                        guard.on_timeout_of(id, now);
                        bytes = guard.poll_transmit_of(id);
                    }
                    if let Some(deadline) = guard.deadline_of(id) {
                        next = next.min(
                            Instant::now() + Duration::try_from(deadline - now).unwrap_or_default(),
                        );
                    }
                }
                bytes
            };
            let Some(bytes) = bytes else { continue };
            let response = match ReadRequest::from_tcp(&bytes) {
                Ok(request) if request.unit() == meter.unit => {
                    match timeout(meter.timeout, exchange(port, &request, timing)).await {
                        Ok(Ok(bytes)) => Some(bytes),
                        Ok(Err(error)) => return error,
                        Err(_) => None,
                    }
                }
                _ => None,
            };
            let mut guard = registry.lock().await;
            let now = OffsetDateTime::now_utc();
            if response.is_some_and(|bytes| guard.on_bytes(id, &bytes, now).is_ok()) {
                next = Instant::now();
            } else {
                guard.on_link(id, LinkState::Stale, now);
                // Allow late bytes to drain before the same unit can be queried again.
                *retry = Instant::now() + meter.timeout.max(Duration::from_secs(1));
                next = next.min(*retry);
            }
        }
        tokio::time::sleep_until(next).await;
    }
}

/// Read a single commissioned meter's common identity before the live bus owner starts.
pub(super) async fn inspect(bus: &SerialBus, unit: u8) -> Option<Identity> {
    timeout(Duration::from_secs(2), async {
        let mut port = open(bus).ok()?;
        let mut probe = Probe::new(unit);
        while let Some(request) = probe.request() {
            let request = ReadRequest::from_tcp(&request.encode()).ok()?;
            let bytes = exchange(&mut port, &request, Timing::for_bus(bus))
                .await
                .ok()?;
            let (response, _) = frame::decode(&bytes).ok()??;
            probe.answer(&response.body);
        }
        probe
            .found()
            .filter(|identity| identity.connection_meter)
            .cloned()
    })
    .await
    .ok()
    .flatten()
}

/// Discard buffered and late bytes until the tty has been quiet for t3.5.
async fn wait_for_idle(port: &mut SerialStream, gap: Duration) -> io::Result<()> {
    let mut buf = [0_u8; 256];
    while let Ok(read) = timeout(gap, port.read(&mut buf)).await {
        if read? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
    }
    Ok(())
}

async fn exchange(
    port: &mut SerialStream,
    request: &ReadRequest,
    timing: Timing,
) -> io::Result<Vec<u8>> {
    wait_for_idle(port, timing.frame_gap).await?;
    port.write_all(&request.encode()).await?;
    loop {
        // A tty delivers chunks. Apply t1.5 to observable gaps within an incomplete
        // response and t3.5 to frame boundaries. The outer deadline also bounds noise.
        let mut frame = [0; 256];
        let mut len = port.read(&mut frame).await?;
        if len == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        loop {
            match request.response_len(&frame[..len]) {
                Ok(Some(expected)) if len == expected => {
                    // Additional bytes without a frame gap invalidate this reply.
                    let mut extra = [0; 1];
                    match timeout(timing.frame_gap, port.read_exact(&mut extra)).await {
                        Err(_) => {
                            if let Ok(response) = request.decode_response(&frame[..len]) {
                                return Ok(response);
                            }
                        }
                        Ok(result) => {
                            result?;
                        }
                    }
                    break;
                }
                Err(_) => break,
                Ok(Some(expected)) if len > expected => break,
                _ if len == frame.len() => break,
                _ => {}
            }
            match timeout(timing.character_gap, port.read(&mut frame[len..])).await {
                Ok(read) => {
                    let n = read?;
                    if n == 0 {
                        return Err(io::ErrorKind::UnexpectedEof.into());
                    }
                    len += n;
                }
                Err(_) => break,
            }
        }
        wait_for_idle(port, timing.frame_gap).await?;
    }
}

#[cfg(all(test, unix, feature = "house"))]
mod tests;
