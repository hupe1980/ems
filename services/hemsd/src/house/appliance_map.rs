//! A declared read window and one writable control word at address 2.
use tokio_modbus::{ExceptionCode, Request, Response};

pub(super) fn request<const N: usize>(
    request: &Request<'_>,
    words: [u16; N],
    accepted: std::ops::RangeInclusive<u16>,
) -> Result<(Response, Option<u16>), ExceptionCode> {
    match request {
        Request::ReadHoldingRegisters(start, count) => {
            if !(1..=125).contains(count) {
                return Err(ExceptionCode::IllegalDataValue);
            }

            let start = usize::from(*start);
            words
                .get(start..start + usize::from(*count))
                .map(|s| (Response::ReadHoldingRegisters(s.to_vec()), None))
                .ok_or(ExceptionCode::IllegalDataAddress)
        }
        Request::WriteSingleRegister(2, value) => {
            if !accepted.contains(value) {
                return Err(ExceptionCode::IllegalDataValue);
            }
            Ok((Response::WriteSingleRegister(2, *value), Some(*value)))
        }
        Request::WriteMultipleRegisters(_, values) if !(1..=123).contains(&values.len()) => {
            Err(ExceptionCode::IllegalDataValue)
        }
        Request::WriteMultipleRegisters(2, values) if values.len() == 1 => {
            if !accepted.contains(&values[0]) {
                return Err(ExceptionCode::IllegalDataValue);
            }
            Ok((Response::WriteMultipleRegisters(2, 1), Some(values[0])))
        }
        Request::WriteSingleRegister(..) | Request::WriteMultipleRegisters(..) => {
            Err(ExceptionCode::IllegalDataAddress)
        }
        _ => Err(ExceptionCode::IllegalFunction),
    }
}

/// Whole Wh, high word first, on a simulator energy register.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn watt_hours(energy: ems_core::prelude::Energy) -> [u16; 2] {
    let bytes = (energy.get().round().clamp(0.0, f64::from(u32::MAX)) as u32).to_be_bytes();
    [
        u16::from_be_bytes([bytes[0], bytes[1]]),
        u16::from_be_bytes([bytes[2], bytes[3]]),
    ]
}
