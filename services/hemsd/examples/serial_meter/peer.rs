use super::Unit;
use std::future::{Ready, ready};
use std::sync::{Arc, Mutex};
use tokio_modbus::{ExceptionCode, Request, Response, SlaveRequest, server::Service};

pub(super) struct Meters {
    pub grid: Option<Unit>,
    pub feeder: Unit,
}

impl Meters {
    pub fn new() -> Self {
        let mut feeder = Unit::meter();
        feeder.set_serial_number("feeder-001");
        feeder.set_meter(900., 123_000., 0.);
        Self {
            grid: Some(Self::grid("grid-001")),
            feeder,
        }
    }

    pub fn grid(serial: &str) -> Unit {
        let mut unit = Unit::meter();
        unit.set_serial_number(serial);
        unit.set_meter(-375., 45_000., 12_000.);
        unit
    }
}

/// tokio-modbus supplies the slave framing, CRC and service dispatch.
pub(super) struct Peer(pub Arc<Mutex<Meters>>);

impl Service for Peer {
    type Request = SlaveRequest<'static>;
    type Response = Option<Response>;
    type Exception = ExceptionCode;
    type Future = Ready<Result<Self::Response, Self::Exception>>;

    fn call(&self, request: Self::Request) -> Self::Future {
        let meters = self.0.lock().expect("meter image lock");
        let device = match request.slave {
            1 => meters.grid.as_ref(),
            2 => Some(&meters.feeder),
            _ => None,
        };
        ready(match (device, request.request) {
            (None, _) => Ok(None),
            (Some(unit), Request::ReadHoldingRegisters(address, count)) => unit
                .read(address, count)
                .map(|words| Some(Response::ReadHoldingRegisters(words)))
                .map_err(|_| ExceptionCode::IllegalDataAddress),
            _ => Err(ExceptionCode::IllegalFunction),
        })
    }
}
