//! The Modbus TCP far end: `tokio-modbus`'s server over the register images.
//!
//! Deliberately not `ems-drv`'s codec turned round — a server and a client by
//! one hand agree by construction. What is ours here is only which registers
//! exist.

use std::future::{Ready, ready};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::net::TcpListener;
use tokio_modbus::server::tcp::{Server, accept_tcp_connection};
use tokio_modbus::{ExceptionCode, Request, Response, SlaveRequest};

use super::registers::Refusal;
use super::{Physics, UNIT_BATTERY, UNIT_INVERTER, UNIT_METER};

#[derive(Clone)]
struct Units(Arc<Mutex<Physics>>);

impl tokio_modbus::server::Service for Units {
    type Request = SlaveRequest<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = Ready<Result<Response, ExceptionCode>>;

    fn call(&self, request: Self::Request) -> Self::Future {
        let mut physics = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let unit = match request.slave {
            UNIT_INVERTER => &mut physics.inverter,
            UNIT_METER => &mut physics.meter,
            UNIT_BATTERY => &mut physics.battery,
            // What a gateway answers for a unit it does not front: 0x0B.
            _ => return ready(Err(ExceptionCode::GatewayTargetDevice)),
        };
        let answer = match request.request {
            Request::ReadHoldingRegisters(address, count) => unit
                .read(address, count)
                .map(Response::ReadHoldingRegisters),
            Request::WriteMultipleRegisters(address, values) => {
                let count = u16::try_from(values.len()).unwrap_or(u16::MAX);
                unit.write(address, &values)
                    .map(|()| Response::WriteMultipleRegisters(address, count))
            }
            Request::WriteSingleRegister(address, value) => unit
                .write(address, &[value])
                .map(|()| Response::WriteSingleRegister(address, value)),
            _ => return ready(Err(ExceptionCode::IllegalFunction)),
        };
        ready(answer.map_err(|refusal| match refusal {
            Refusal::Address => ExceptionCode::IllegalDataAddress,
            Refusal::Value => ExceptionCode::IllegalDataValue,
        }))
    }
}

/// Serve every unit of the house on `listener` until the task is dropped.
pub(super) async fn serve(listener: TcpListener, physics: Arc<Mutex<Physics>>) {
    let units = Units(physics);
    let on_connected = |stream, peer: SocketAddr| {
        let units = units.clone();
        async move { accept_tcp_connection(stream, peer, move |_| Ok(Some(units.clone()))) }
    };
    let on_error = |error| tracing::debug!(%error, "a Modbus connection ended");
    if let Err(error) = Server::new(listener).serve(&on_connected, on_error).await {
        tracing::error!(%error, "the house's Modbus server stopped");
    }
}
