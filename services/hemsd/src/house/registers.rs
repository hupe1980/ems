//! SunSpec register images, written from typed values.
//!
//! The `sunspec` crate parses and does not encode, so the house lays its models
//! out itself — but every offset is that crate's own `Point`, never a number
//! typed here, and the tests read each image back through `Model::parse`. The
//! box reads the same registers through the same crate, so a disagreement
//! between this file and the driver is a disagreement with the specification.

use sunspec::Group;
use sunspec::models::model1::Model1;
use sunspec::models::model103::Model103;
use sunspec::models::model123::Model123;
use sunspec::models::model203::Model203;
use sunspec::models::model802::Model802;

/// Where every unit's chain starts — the common SunSpec base.
pub const BASE: u16 = 40_000;

/// `SunS`.
const MARKER: [u16; 2] = [0x5375, 0x6e53];

/// The identifier that ends a model chain.
const END_OF_CHAIN: u16 = 0xffff;

/// The most registers one Modbus read may ask for.
const MAX_READ: u16 = 125;

/// Why a request is refused, as the Modbus exception it becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `0x02`: the registers are not there, or may not be written.
    Address,
    /// `0x03`: the count or a value is out of range.
    Value,
}

/// One Modbus unit: a SunSpec chain and the registers under it.
#[derive(Debug, Clone)]
pub struct Unit {
    words: Vec<u16>,
    /// Model id → (index of its body, length).
    bodies: Vec<(u16, usize, usize)>,
}

impl Unit {
    /// A chain of the common model followed by `models`, all zeroed.
    fn new(name: &str, models: &[(u16, u16)]) -> Self {
        let mut words = MARKER.to_vec();
        let mut bodies = Vec::new();
        for (id, len) in std::iter::once(&(1, Model1::LEN)).chain(models) {
            words.extend([*id, *len]);
            bodies.push((*id, words.len(), usize::from(*len)));
            words.resize(words.len() + usize::from(*len), 0);
        }
        words.extend([END_OF_CHAIN, 0]);
        let mut unit = Self { words, bodies };
        let common = unit.body_mut(1);
        text(common, Model1::MN.offset, Model1::MN.length, "hems");
        text(common, Model1::MD.offset, Model1::MD.length, name);
        text(
            common,
            Model1::SN.offset,
            Model1::SN.length,
            "virtual-house",
        );
        unit
    }

    /// A three-phase inverter with immediate controls: models 103 and 123.
    #[must_use]
    pub fn inverter() -> Self {
        let mut unit = Self::new(
            "virtual inverter",
            &[(103, Model103::LEN), (123, Model123::LEN)],
        );
        let controls = unit.body_mut(123);
        set(controls, Model123::CONN.offset, 1);
        set(controls, Model123::W_MAX_LIM_PCT.offset, 100);
        unit
    }

    /// A three-phase meter at the grid connection: model 203.
    #[must_use]
    pub fn meter() -> Self {
        Self::new("virtual meter", &[(203, Model203::LEN)])
    }

    /// A battery: model 802.
    #[must_use]
    pub fn battery() -> Self {
        Self::new("virtual battery", &[(802, Model802::LEN)])
    }

    fn range(&self, id: u16) -> (usize, usize) {
        self.bodies
            .iter()
            .find(|(model, ..)| *model == id)
            .map_or((0, 0), |(_, start, len)| (*start, *len))
    }

    /// The body of model `id`; empty where the unit does not publish it.
    #[must_use]
    pub fn body(&self, id: u16) -> &[u16] {
        let (start, len) = self.range(id);
        &self.words[start..start + len]
    }

    fn body_mut(&mut self, id: u16) -> &mut [u16] {
        let (start, len) = self.range(id);
        &mut self.words[start..start + len]
    }

    fn span(&self, address: u16, count: usize) -> Result<std::ops::Range<usize>, Refusal> {
        let start = usize::from(address.checked_sub(BASE).ok_or(Refusal::Address)?);
        let end = start + count;
        if end > self.words.len() {
            return Err(Refusal::Address);
        }
        Ok(start..end)
    }

    /// Function code 3.
    ///
    /// # Errors
    /// [`Refusal`] for a count no device answers or registers that do not exist.
    pub fn read(&self, address: u16, count: u16) -> Result<Vec<u16>, Refusal> {
        if count == 0 || count > MAX_READ {
            return Err(Refusal::Value);
        }
        Ok(self.words[self.span(address, usize::from(count))?].to_vec())
    }

    /// Function code 16: only model 123's writable points, and only values an
    /// inverter would take.
    ///
    /// # Errors
    /// [`Refusal`] for anything else — as a real inverter answers.
    pub fn write(&mut self, address: u16, values: &[u16]) -> Result<(), Refusal> {
        let span = self.span(address, values.len())?;
        let (start, len) = self.range(123);
        let writable = start..start + usize::from(Model123::V_AR_PCT_ENA.offset) + 1;
        if len == 0 || values.is_empty() || span.start < writable.start || span.end > writable.end {
            return Err(Refusal::Address);
        }
        let mut next = self.words.clone();
        next[span].copy_from_slice(values);
        let body = &next[start..];
        if body[usize::from(Model123::W_MAX_LIM_PCT.offset)] > 100
            || body[usize::from(Model123::W_MAX_LIM_ENA.offset)] > 1
            || body[usize::from(Model123::CONN.offset)] > 1
        {
            return Err(Refusal::Value);
        }
        self.words = next;
        Ok(())
    }

    /// The ceiling the box has written, as a fraction of the rating: `None`
    /// while no limit is in force, zero while the inverter is disconnected.
    #[must_use]
    pub fn curtailment(&self) -> Option<f64> {
        let body = self.body(123);
        let at = |offset: u16| body.get(usize::from(offset)).copied().unwrap_or(0);
        if at(Model123::CONN.offset) == 0 {
            return Some(0.0);
        }
        (at(Model123::W_MAX_LIM_ENA.offset) == 1)
            .then(|| f64::from(at(Model123::W_MAX_LIM_PCT.offset)) / 100.0)
    }

    /// What the inverter feeds in (W, non-negative) and has fed in (Wh).
    pub fn set_inverter(&mut self, watts: f64, watt_hours: f64) {
        let body = self.body_mut(103);
        set(body, Model103::W.offset, signed(watts));
        set(body, Model103::HZ.offset, 5_000);
        set(body, Model103::HZ_SF.offset, signed(-2.0));
        wide(body, Model103::WH.offset, watt_hours);
        set(body, Model103::TMP_CAB.offset, 350);
        set(body, Model103::TMP_SF.offset, signed(-1.0));
        // `MPPT`.
        set(body, Model103::ST.offset, 4);
    }

    /// What crosses the connection point (W, import positive) and the two
    /// counters (Wh).
    pub fn set_meter(&mut self, watts: f64, imported_wh: f64, exported_wh: f64) {
        let body = self.body_mut(203);
        set(body, Model203::W.offset, signed(watts));
        set(body, Model203::HZ.offset, 5_000);
        set(body, Model203::HZ_SF.offset, signed(-2.0));
        wide(body, Model203::TOT_WH_IMP.offset, imported_wh);
        wide(body, Model203::TOT_WH_EXP.offset, exported_wh);
    }

    /// The battery's state of charge, 0–1.
    pub fn set_battery(&mut self, soc: f64) {
        let body = self.body_mut(802);
        set(body, Model802::SOC.offset, signed(soc * 100.0));
    }
}

fn set(body: &mut [u16], offset: u16, value: u16) {
    if let Some(word) = body.get_mut(usize::from(offset)) {
        *word = value;
    }
}

/// A 32-bit counter, high word first.
fn wide(body: &mut [u16], offset: u16, value: f64) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to the counter's range on the same line"
    )]
    let value = value.clamp(0.0, f64::from(u32::MAX)).round() as u32;
    set(body, offset, (value >> 16) as u16);
    set(body, offset + 1, (value & 0xffff) as u16);
}

/// A signed 16-bit value as the register that carries it.
fn signed(value: f64) -> u16 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "clamped to an i16 on the same line"
    )]
    let value = value
        .clamp(f64::from(i16::MIN), f64::from(i16::MAX))
        .round() as i16;
    value.cast_unsigned()
}

/// A string point: two characters a register, NUL-padded.
fn text(body: &mut [u16], offset: u16, length: u16, value: &str) {
    let mut bytes = value.bytes().chain(std::iter::repeat(0));
    for i in 0..length {
        let (high, low) = (bytes.next().unwrap_or(0), bytes.next().unwrap_or(0));
        set(body, offset + i, u16::from_be_bytes([high, low]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunspec::Model;

    #[test]
    fn every_image_parses_as_the_model_it_claims_to_be() {
        let mut inverter = Unit::inverter();
        inverter.set_inverter(4_321.0, 70_000.0);
        let read = Model103::parse(inverter.body(103)).expect("model 103");
        assert_eq!((read.w, read.w_sf, read.wh), (4_321, 0, 70_000));
        let controls = Model123::parse(inverter.body(123)).expect("model 123");
        assert_eq!(controls.w_max_lim_pct, 100);

        let mut meter = Unit::meter();
        meter.set_meter(-2_500.0, 1_000.0, 90_000.0);
        let read = Model203::parse(meter.body(203)).expect("model 203");
        assert_eq!(
            (read.w, read.tot_wh_imp, read.tot_wh_exp),
            (-2_500, 1_000, 90_000)
        );

        let mut battery = Unit::battery();
        battery.set_battery(0.42);
        assert_eq!(Model802::parse(battery.body(802)).expect("802").soc, 42);

        let common = Model1::parse(meter.body(1)).expect("model 1");
        assert_eq!(common.md, "virtual meter");
    }

    #[test]
    fn the_chain_walks_from_the_marker_to_the_sentinel() {
        let unit = Unit::inverter();
        assert_eq!(unit.read(BASE, 2), Ok(MARKER.to_vec()));
        let mut address = BASE + 2;
        let mut seen = Vec::new();
        loop {
            let header = unit.read(address, 2).expect("a header");
            if header[0] == END_OF_CHAIN {
                break;
            }
            seen.push(header[0]);
            address += 2 + header[1];
        }
        assert_eq!(seen, [1, 103, 123]);
    }

    #[test]
    fn a_limit_is_taken_where_the_model_puts_it_and_nowhere_else() {
        let mut unit = Unit::inverter();
        let controls = BASE + 2 + 2 + Model1::LEN + 2 + Model103::LEN + 2;
        let limit = controls + Model123::W_MAX_LIM_PCT.offset;
        assert_eq!(unit.curtailment(), None);
        unit.write(limit, &[40, 0, 0, 0, 1]).expect("a limit");
        assert_eq!(unit.curtailment(), Some(0.4));

        assert_eq!(unit.write(limit, &[140]), Err(Refusal::Value));
        assert_eq!(unit.write(BASE + 10, &[1]), Err(Refusal::Address));
        assert_eq!(
            unit.curtailment(),
            Some(0.4),
            "a refused write changes nothing"
        );

        // `Conn` is the register a write three too low lands on.
        unit.write(controls, &[40, 0, 0, 0, 1])
            .expect("it is writable");
        assert_eq!(
            unit.curtailment(),
            Some(0.0),
            "and it disconnects the inverter"
        );
    }

    #[test]
    fn a_read_no_device_answers_is_refused() {
        let unit = Unit::meter();
        assert_eq!(unit.read(BASE, 126), Err(Refusal::Value));
        assert_eq!(unit.read(BASE - 1, 2), Err(Refusal::Address));
        assert_eq!(unit.read(BASE + 5_000, 2), Err(Refusal::Address));
    }
}
