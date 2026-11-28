//! Read-only, single-telegram wired M-Bus heat meters (EN 13757).
use mbus_kit::{
    Application, Function, Header, Kind, Manufacturer, Medium, Unit,
    wired::{self, Frame},
};
use rust_decimal::Decimal;

/// Largest EN 13757 wired frame, including its envelope.
pub const MAX_FRAME: usize = 261;

/// A primary address bound to the manufacturer and eight digits on the meter.
#[derive(Clone, Copy, Debug)]
pub struct HeatMeter {
    address: u8,
    manufacturer: Manufacturer,
    id: u32,
}

/// Why this reply cannot supply a current delivered-heat register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The wire frame is malformed or incomplete.
    #[error("invalid M-Bus frame")]
    Frame,
    /// A different address, identity or medium answered.
    #[error("unexpected heat meter")]
    Identity,
    /// A device status, unsupported profile or ambiguous register prevents use.
    #[error("unusable heat register")]
    Register,
}

impl HeatMeter {
    /// Bind a unicast address, three-letter FLAG code and eight decimal digits.
    #[must_use]
    pub fn new(address: u8, manufacturer: &str, id: &str) -> Option<Self> {
        if address > 250 || id.len() != 8 || !id.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(Self {
            address,
            manufacturer: Manufacturer::from_letters(manufacturer.as_bytes().try_into().ok()?)?,
            id: id.parse().ok()?,
        })
    }

    /// Reset the link counter, then request class-2 data with FCB set.
    /// The caller must receive a single-byte acknowledgement between requests.
    #[must_use]
    pub fn requests(self) -> [[u8; 5]; 2] {
        [
            wired::snd_nke(self.address),
            wired::req_ud2(self.address, true),
        ]
        .map(|frame| {
            let mut bytes = [0; 5];
            frame
                .encode(&mut bytes)
                .expect("a short frame is five bytes");
            bytes
        })
    }

    /// Decode one complete reply, returning exact cumulative thermal Wh.
    ///
    /// # Errors
    /// Rejects foreign replies, device errors, partial readouts and unusable registers.
    pub fn delivered_wh(self, bytes: &[u8]) -> Result<Decimal, ReadError> {
        let Some((Frame::Long { c, a, ci, data }, used)) =
            Frame::parse(bytes).map_err(|_| ReadError::Frame)?
        else {
            return Err(ReadError::Frame);
        };
        if used != bytes.len() || !wired::control::is_response(c) || ci != 0x72 {
            return Err(ReadError::Frame);
        }
        let app = Application::parse(ci, data).map_err(|_| ReadError::Frame)?;
        let Header::Long(header) = app.header else {
            return Err(ReadError::Identity);
        };
        if a != self.address
            || header.address.manufacturer != self.manufacturer
            || header.address.id_number() != Some(self.id)
            || !matches!(header.address.medium, Medium::HEAT | Medium::HEAT_INLET)
        {
            return Err(ReadError::Identity);
        }
        if header.short.status != 0 || header.short.security().mode != 0 {
            return Err(ReadError::Register);
        }
        let mut records = app.records().ok_or(ReadError::Register)?;
        let mut total = None;
        for record in records.by_ref() {
            let r = record.map_err(|_| ReadError::Frame)?;
            if r.dib.function != Function::Instantaneous
                || r.dib.storage != 0
                || r.dib.tariff != 0
                || r.dib.subunit != 0
                || r.description().kind != Kind::Energy
                || r.vib.modifiers().next().is_some()
            {
                continue;
            }
            let q = r.quantity().ok_or(ReadError::Register)?;
            if q.unit != Some(Unit::Wh) || q.mantissa < 0 || total.is_some() {
                return Err(ReadError::Register);
            }
            let (value, scale) = if q.exponent >= 0 {
                (q.to_scale(0).ok_or(ReadError::Register)?, 0)
            } else {
                (q.mantissa, u32::from(q.exponent.unsigned_abs()))
            };
            total = Some(
                Decimal::try_from_i128_with_scale(value, scale).map_err(|_| ReadError::Register)?,
            );
        }
        if records.more_follows() {
            return Err(ReadError::Register);
        }
        total.ok_or(ReadError::Register)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meter() -> HeatMeter {
        HeatMeter::new(17, "KAM", "06855817").unwrap()
    }

    fn captured() -> Vec<u8> {
        include_str!("../tests/fixtures/mbus/kamstrup-601.hex")
            .trim()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect()
    }

    fn frame(records: &[u8]) -> Vec<u8> {
        let mut data = captured()[7..19].to_vec();
        data.extend(records);
        let mut out = [0; MAX_FRAME];
        let len = Frame::Long {
            c: 8,
            a: 17,
            ci: 0x72,
            data: &data,
        }
        .encode(&mut out)
        .unwrap();
        out[..len].to_vec()
    }

    #[test]
    fn commissioned_identity_and_capture() {
        // SR-167, SR-168. Independent libmbus MULTICAL 601 reference XML.
        let bytes = captured();
        assert_eq!(
            meter().delivered_wh(&bytes).unwrap().to_string(),
            "37351000"
        );
        for (address, manufacturer, id) in [
            (16, "KAM", "06855817"),
            (17, "KAM", "06855818"),
            (17, "ABB", "06855817"),
        ] {
            assert_eq!(
                HeatMeter::new(address, manufacturer, id)
                    .unwrap()
                    .delivered_wh(&bytes),
                Err(ReadError::Identity)
            );
        }
        let changed = |index: usize, value| {
            let mut reply = bytes.clone();
            reply[index] = value;
            let n = reply.len();
            reply[n - 2] = reply[4..n - 2]
                .iter()
                .fold(0u8, |acc, b| acc.wrapping_add(*b));
            reply
        };
        for control in [0x08, 0x18, 0x28, 0x38] {
            assert_eq!(
                meter().delivered_wh(&changed(4, control)),
                Ok(Decimal::from(37_351_000))
            );
        }
        for (index, value) in [(4, 0x48), (4, 0x88), (4, 0x53), (6, 0x78), (14, 7), (16, 4)] {
            assert!(meter().delivered_wh(&changed(index, value)).is_err());
        }
        assert_eq!(
            meter().delivered_wh(&[bytes.as_slice(), &bytes].concat()),
            Err(ReadError::Frame)
        );
        for len in 0..bytes.len() {
            assert!(meter().delivered_wh(&bytes[..len]).is_err());
        }
    }

    #[test]
    fn current_exact_and_unambiguous() {
        // SR-168. Zero and fractional Wh, duplicate, archive, continuation and invalid tail.
        for (records, want) in [(&[1, 3, 0][..], "0"), (&[1, 0, 7], "0.007")] {
            assert_eq!(
                meter().delivered_wh(&frame(records)).unwrap().to_string(),
                want
            );
        }
        for records in [
            &[1, 3, 1, 1, 3, 2][..],
            &[1, 3, 1, 0x1f],
            &[0x41, 3, 0],
            &[1, 3, 1, 4],
            &[1, 3, 255],
            &[1, 0x83, 0x3c, 1],
        ] {
            assert!(meter().delivered_wh(&frame(records)).is_err());
        }
    }
}
