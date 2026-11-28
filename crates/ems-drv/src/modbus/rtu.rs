//! Unicast register reads on a Modbus RTU bus. The bus owner delimits frames
//! by silence and keeps only one request outstanding. The transaction ID stays
//! local so the same driver can process a reply received over TCP or serial.

use super::frame::{FrameError, MAX_REGISTERS_PER_READ, function, tcp_adu};

/// A validated register read and the local transaction that issued it.
#[derive(Debug, Clone, Copy)]
pub struct ReadRequest {
    transaction: u16,
    unit: u8,
    function: u8,
    address: u16,
    count: u16,
}

impl ReadRequest {
    /// Adapt an existing driver's unicast holding- or input-register request.
    pub fn from_tcp(frame: &[u8]) -> Result<Self, FrameError> {
        let &[
            tx_hi,
            tx_lo,
            0,
            0,
            0,
            6,
            unit,
            function,
            addr_hi,
            addr_lo,
            n_hi,
            n_lo,
        ] = frame
        else {
            return Err(FrameError::Malformed("register read header"));
        };
        if !(1..=247).contains(&unit) {
            return Err(FrameError::Malformed("RTU unit outside 1..=247"));
        }
        if !matches!(function, function::READ_HOLDING | function::READ_INPUT) {
            return Err(FrameError::Malformed("RTU request is not a register read"));
        }
        let address = u16::from_be_bytes([addr_hi, addr_lo]);
        let count = u16::from_be_bytes([n_hi, n_lo]);
        if !(1..=MAX_REGISTERS_PER_READ).contains(&count)
            || address.checked_add(count - 1).is_none()
        {
            return Err(FrameError::Malformed("RTU register range"));
        }
        Ok(Self {
            transaction: u16::from_be_bytes([tx_hi, tx_lo]),
            unit,
            function,
            address,
            count,
        })
    }

    /// Server address for this transaction.
    pub fn unit(&self) -> u8 {
        self.unit
    }

    /// Unit, register read and CRC, ready for the serial port.
    pub fn encode(&self) -> [u8; 8] {
        let [addr_hi, addr_lo] = self.address.to_be_bytes();
        let [n_hi, n_lo] = self.count.to_be_bytes();
        let mut adu = [self.unit, self.function, addr_hi, addr_lo, n_hi, n_lo, 0, 0];
        let crc = crc16(&adu[..6]);
        adu[6..].copy_from_slice(&crc.to_le_bytes());
        adu
    }

    /// Expected frame length, including CRC, once the prefix identifies the
    /// response. Reject a different unit, function or register count early.
    pub fn response_len(&self, prefix: &[u8]) -> Result<Option<usize>, FrameError> {
        let Some((&unit, rest)) = prefix.split_first() else {
            return Ok(None);
        };
        if unit != self.unit {
            return Err(FrameError::Malformed("unexpected RTU unit"));
        }
        let Some((&function, data)) = rest.split_first() else {
            return Ok(None);
        };
        if function == self.function | 0x80 {
            return Ok(Some(5));
        }
        if function != self.function {
            return Err(FrameError::Malformed("unexpected RTU function"));
        }
        let Some(&bytes) = data.first() else {
            return Ok(None);
        };
        if u16::from(bytes) != self.count * 2 {
            return Err(FrameError::Malformed("unexpected RTU register count"));
        }
        Ok(Some(5 + usize::from(bytes)))
    }

    /// Validate a complete reply before passing its PDU to the device driver.
    pub fn decode_response(&self, adu: &[u8]) -> Result<Vec<u8>, FrameError> {
        if self.response_len(adu)? != Some(adu.len()) {
            return Err(FrameError::Malformed("RTU response length"));
        }
        let (data, checksum) = adu.split_at(adu.len() - 2);
        if crc16(data).to_le_bytes() != checksum {
            return Err(FrameError::Malformed("RTU checksum mismatch"));
        }
        Ok(tcp_adu(self.transaction, self.unit, &data[1..]))
    }
}

// Modbus Serial Line V1.02, Appendix B. The low CRC byte is transmitted first.
fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xffff;
    for &byte in data {
        crc ^= u16::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xa001
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modbus::frame::{ResponseBody, decode};

    const READ: [u8; 12] = [0xab, 0xcd, 0, 0, 0, 6, 0x11, 3, 0, 0x6b, 0, 3];
    const REPLY: [u8; 11] = [0x11, 3, 6, 0xae, 0x41, 0x56, 0x52, 0x43, 0x40, 0x49, 0xad];

    #[test]
    fn rtu_register_exchange_preserves_the_driver_transaction() {
        let req = ReadRequest::from_tcp(&READ).unwrap();
        assert_eq!(req.encode(), [0x11, 3, 0, 0x6b, 0, 3, 0x76, 0x87]);
        let tcp = req.decode_response(&REPLY).unwrap();
        let (reply, used) = decode(&tcp).unwrap().unwrap();
        assert_eq!(used, tcp.len());
        assert_eq!(reply.unit, 0x11);
        assert_eq!(reply.transaction, 0xabcd);
        assert_eq!(
            reply.body,
            ResponseBody::Registers(vec![0xae41, 0x5652, 0x4340])
        );
        let exception = [0x11, 0x83, 2, 0xc1, 0x34];
        assert_eq!(req.response_len(&exception[..2]), Ok(Some(5)));
        let tcp = req.decode_response(&exception).unwrap();
        assert_eq!(
            decode(&tcp).unwrap().unwrap().0.body,
            ResponseBody::Exception {
                function: 3,
                code: 2
            }
        );
    }

    #[test]
    fn incomplete_damaged_or_unrelated_rtu_replies_yield_no_frame() {
        let req = ReadRequest::from_tcp(&READ).unwrap();
        for cut in 0..REPLY.len() {
            assert!(req.decode_response(&REPLY[..cut]).is_err());
            assert_eq!(
                req.response_len(&REPLY[..cut]).unwrap(),
                (cut >= 3).then_some(REPLY.len())
            );
        }
        for i in 0..REPLY.len() {
            let mut changed = REPLY;
            changed[i] ^= 1;
            assert!(req.decode_response(&changed).is_err());
        }
        let mut extra = REPLY.to_vec();
        extra.push(0);
        assert!(req.decode_response(&extra).is_err());
        assert!(req.response_len(&[0x12]).is_err());
        assert!(req.response_len(&[0x11, 4]).is_err());
        assert!(req.response_len(&[0x11, 3, 250]).is_err());
    }

    #[test]
    fn serial_gateway_only_admits_well_formed_unicast_reads() {
        for (i, value) in [
            (3, 1),
            (5, 5),
            (6, 0),
            (6, 248),
            (6, 255),
            (7, 16),
            (11, 0),
            (11, 126),
        ] {
            let mut bad = READ;
            bad[i] = value;
            assert!(ReadRequest::from_tcp(&bad).is_err());
        }
        let mut end = READ;
        end[8..10].copy_from_slice(&u16::MAX.to_be_bytes());
        assert!(ReadRequest::from_tcp(&end).is_err());
        end[11] = 1;
        end[7] = function::READ_INPUT;
        assert_eq!(ReadRequest::from_tcp(&end).unwrap().unit(), 0x11);
    }
}
