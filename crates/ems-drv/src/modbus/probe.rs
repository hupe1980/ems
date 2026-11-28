//! Asking one address whether a SunSpec device lives there.
//!
//! Commissioning's question, not the driver's: an installer names a host, and
//! the box says what — if anything — answered. It reads the `SunS` marker at the
//! three base addresses SunSpec allows (40000, 50000 and 0) and then the common model
//! (model 1: manufacturer, model, version, serial number), and the model headers that follow. It
//! never writes.
//!
//! Sans-I/O like the rest of this crate: [`Probe::request`] is the next frame to
//! send, [`Probe::answer`] takes what came back, and whoever owns the socket
//! decides how long to wait.

use super::frame::{Request, RequestBody, ResponseBody, Space};
use super::scan::{BASES, Discovery, MARKER, ModelMap};
use sunspec::Group;
use sunspec::models::{
    model103::Model103, model123::Model123, model203::Model203, model802::Model802,
};

/// The common model's header and the fixed part of its body: `ID`, `L`, then
/// `Mn` (16 registers), `Md` (16), `Opt` (8), `Vr` (8) and `SN` (16).
const COMMON_REGISTERS: u16 = 2 + 16 + 16 + 8 + 8 + 16;

/// What a SunSpec device says about itself in its common model.
///
/// Everything here came off the wire from a device nobody has authenticated —
/// Modbus has no way to — so it is a claim to show an installer, never a fact to
/// act on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Identity {
    /// The base address the `SunS` marker was found at.
    pub base: u16,
    /// A supported PV model in the device’s model chain.
    pub inverter_model: Option<u16>,
    /// The driver can express a production ceiling with model 123.
    pub has_immediate_controls: bool,
    /// The live driver supports the three-phase model 203 meter.
    pub connection_meter: bool,
    /// A supported model 802 battery measurement block.
    pub storage_meter: bool,
    /// `Mn`, the manufacturer.
    pub manufacturer: String,
    /// `Md`, the model.
    pub model: String,
    /// `Vr`, the firmware version.
    pub version: String,
    /// `SN`, the serial number.
    pub serial: String,
}

impl Identity {
    /// Whether this unit advertises both PV measurement and production limiting.
    #[must_use]
    pub fn supports_pv_driver(&self) -> bool {
        self.inverter_model.is_some() && self.has_immediate_controls
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Looking for the marker at `BASES[i]`.
    Marker(usize),
    /// Reading the common model that follows the marker at `base`.
    Common(u16),
    Headers,
    /// Finished, one way or the other.
    Done,
}

/// One probe of one unit at one address.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    unit: u8,
    state: State,
    found: Option<Identity>,
    chain: Discovery,
    models: ModelMap,
}

impl Probe {
    /// A probe of Modbus unit `unit` that has not started.
    #[must_use]
    pub fn new(unit: u8) -> Self {
        Self {
            unit,
            state: State::Marker(0),
            found: None,
            chain: Discovery::new(),
            models: ModelMap::default(),
        }
    }

    /// The next frame to send, or `None` once the probe is finished.
    ///
    /// Header reads use the live driver’s bounded model walk.
    #[must_use]
    pub fn request(&self) -> Option<Request> {
        let (transaction, address, count) = match self.state {
            State::Marker(i) => (u16::try_from(i).unwrap_or(0) + 1, *BASES.get(i)?, 2),
            State::Common(base) => (9, base.checked_add(2)?, COMMON_REGISTERS),
            State::Headers => {
                let (address, count, _) = self.chain.next_step(&self.models)?;
                (address, address, count)
            }
            State::Done => return None,
        };
        Some(Request {
            transaction,
            unit: self.unit,
            body: RequestBody::Read {
                space: Space::Holding,
                address,
                count,
            },
        })
    }

    /// What the device answered to the last [`request`](Self::request).
    ///
    /// A refusal or a wrong marker moves to the next base; anything else that is
    /// not what was asked for ends the probe with nothing found.
    pub fn answer(&mut self, body: &ResponseBody) {
        self.state = match (self.state, body) {
            (State::Marker(i), ResponseBody::Registers(regs)) if regs.as_slice() == MARKER => {
                self.chain.saw_marker(BASES[i], regs, &mut self.models);
                State::Common(BASES[i])
            }
            (State::Marker(i), _) if i + 1 < BASES.len() => State::Marker(i + 1),
            (State::Common(base), ResponseBody::Registers(regs)) => {
                self.found = common(base, regs);
                if self.found.is_some() {
                    self.chain.saw_header(base + 2, regs, &mut self.models);
                    State::Headers
                } else {
                    State::Done
                }
            }
            (State::Headers, body) => {
                let Some((address, _, purpose)) = self.chain.next_step(&self.models) else {
                    return;
                };
                let complete = match body {
                    ResponseBody::Registers(regs) => {
                        self.chain.saw_header(address, regs, &mut self.models);
                        regs.as_slice() == [0xffff, 0]
                    }
                    ResponseBody::Exception {
                        function: 3,
                        code: 2,
                    } => {
                        self.chain.refused(purpose, &mut self.models);
                        true
                    }
                    _ => {
                        self.state = State::Done;
                        return;
                    }
                };
                if complete && let Some(identity) = &mut self.found {
                    identity.storage_meter = self.models.kind() == super::Kind::Battery
                        && self
                            .models
                            .iter()
                            .any(|(id, _, len)| id == 802 && len >= Model802::LEN);
                    identity.connection_meter = self.models.kind() == super::Kind::Meter
                        && self
                            .models
                            .iter()
                            .any(|(id, _, len)| id == 203 && len == Model203::LEN);
                }
                if complete
                    && self.models.kind() == super::Kind::Inverter
                    && let Some(identity) = &mut self.found
                {
                    identity.has_immediate_controls = self
                        .models
                        .iter()
                        .any(|(id, _, len)| id == 123 && len == Model123::LEN);
                    identity.inverter_model = self.models.iter().find_map(|(id, _, len)| {
                        ((101..=103).contains(&id) && len == Model103::LEN).then_some(id)
                    });
                }
                State::Headers
            }
            _ => State::Done,
        };
    }

    /// Body of the confirmed three-phase meter model.
    pub(super) fn meter_block(&self) -> Option<(u16, u16)> {
        self.found
            .as_ref()
            .filter(|identity| identity.connection_meter)?;
        self.models
            .iter()
            .find_map(|(id, address, len)| (id == 203).then_some((address, len)))
    }

    /// What was found, once [`request`](Self::request) has nothing left to ask.
    #[must_use]
    pub fn found(&self) -> Option<&Identity> {
        self.found.as_ref()
    }
}

/// Read the common model out of the registers that follow the marker.
fn common(base: u16, regs: &[u16]) -> Option<Identity> {
    // Model 1, and long enough to hold what is about to be read out of it.
    if regs.len() < usize::from(COMMON_REGISTERS) || regs[0] != 1 || regs[1] < COMMON_REGISTERS - 2
    {
        return None;
    }
    Some(Identity {
        base,
        inverter_model: None,
        has_immediate_controls: false,
        connection_meter: false,
        storage_meter: false,
        manufacturer: text(&regs[2..18]),
        model: text(&regs[18..34]),
        version: text(&regs[42..50]),
        serial: text(&regs[50..66]),
    })
}

/// A SunSpec string: two bytes a register, NUL-padded.
///
/// Printable ASCII only, and the rest dropped: this text is shown to an
/// installer and written to a log, and it was chosen by whatever answered.
fn text(regs: &[u16]) -> String {
    regs.iter()
        .flat_map(|r| r.to_be_bytes())
        .take_while(|b| *b != 0)
        .filter(|b| b.is_ascii_graphic() || *b == b' ')
        .map(char::from)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str, registers: usize) -> Vec<u16> {
        let mut bytes = s.as_bytes().to_vec();
        bytes.resize(registers * 2, 0);
        bytes
            .chunks(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect()
    }

    fn common_model() -> Vec<u16> {
        let mut regs = vec![1, 66];
        regs.extend(words("Fronius", 16));
        regs.extend(words("Symo 10.0-3-M", 16));
        regs.extend(words("", 8));
        regs.extend(words("3.24.1-2", 8));
        regs.extend(words("29210000123", 16));
        regs
    }

    fn address(request: &Request) -> u16 {
        match request.body {
            RequestBody::Read { address, .. } => address,
            RequestBody::Write { .. } => panic!("a probe never writes"),
        }
    }

    #[test]
    fn a_device_at_the_second_base_is_found_and_named() {
        let mut probe = Probe::new(1);
        assert_eq!(address(&probe.request().unwrap()), 40_000);
        probe.answer(&ResponseBody::Exception {
            function: 3,
            code: 2,
        });
        assert_eq!(address(&probe.request().unwrap()), 50_000);
        probe.answer(&ResponseBody::Registers(MARKER.to_vec()));
        assert_eq!(address(&probe.request().unwrap()), 50_002);
        probe.answer(&ResponseBody::Registers(common_model()));
        assert_eq!(address(&probe.request().unwrap()), 50_070);
        probe.answer(&ResponseBody::Registers(vec![103, Model103::LEN]));
        probe.answer(&ResponseBody::Registers(vec![0xffff, 0]));
        assert!(probe.request().is_none());
        assert!(!probe.found().unwrap().supports_pv_driver());
        assert_eq!(
            probe.found(),
            Some(&Identity {
                base: 50_000,
                inverter_model: Some(103),
                has_immediate_controls: false,
                connection_meter: false,
                storage_meter: false,
                manufacturer: "Fronius".into(),
                model: "Symo 10.0-3-M".into(),
                version: "3.24.1-2".into(),
                serial: "29210000123".into(),
            })
        );
    }

    #[test]
    fn something_that_is_not_sunspec_is_asked_three_times_and_no_more() {
        let mut probe = Probe::new(1);
        let mut asked = 0;
        while probe.request().is_some() {
            asked += 1;
            // A register map of some other kind: it answers, with other numbers.
            probe.answer(&ResponseBody::Registers(vec![0x1234, 0x5678]));
        }
        assert_eq!(asked, 3);
        assert!(probe.found().is_none());
    }

    #[test]
    fn a_marker_followed_by_some_other_model_is_not_a_device() {
        let mut probe = Probe::new(1);
        probe.answer(&ResponseBody::Registers(MARKER.to_vec()));
        let mut regs = common_model();
        regs[0] = 103;
        probe.answer(&ResponseBody::Registers(regs));
        assert!(probe.request().is_none());
        assert!(probe.found().is_none());
    }

    #[test]
    fn what_the_device_calls_itself_cannot_carry_a_control_character() {
        let mut regs = common_model();
        regs[2] = u16::from_be_bytes(*b"A\n");
        regs[3] = u16::from_be_bytes([0x1b, b'B']);
        let identity = common(40_000, &regs).unwrap();
        assert!(identity.manufacturer.starts_with("AB"));
    }

    #[test]
    fn a_probe_only_ever_reads() {
        let mut probe = Probe::new(7);
        while let Some(request) = probe.request() {
            assert!(matches!(request.body, RequestBody::Read { .. }));
            assert_eq!(request.unit, 7);
            probe.answer(&ResponseBody::Registers(MARKER.to_vec()));
        }
    }
}
