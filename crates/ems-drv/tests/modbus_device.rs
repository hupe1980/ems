//! The driver against a device that answers.
//!
//! Everything here is bytes and a clock. There is no socket, no runtime and no
//! sleep: the "device" is a function from a request to a reply, so a discovery
//! walk, a poll and a silent inverter are all ordinary assertions.
//!
//! This is what the sans-I/O contract is *for*, and it is what proves
//! `ems_drv::Driver` is a trait somebody can actually implement.

#![cfg(feature = "modbus")]

use ems_core::prelude::{AssetId, Current, Power};
use ems_core::setpoint::Command;
use ems_drv::modbus::frame::{self, RequestBody, ResponseBody};
use ems_drv::modbus::{Cadence, Kind, SunSpec};
use ems_drv::{Driver, DriverError, DriverEvent, LinkState};
use time::macros::datetime;
use time::{Duration, OffsetDateTime};

const T0: OffsetDateTime = datetime!(2026-05-15 12:00:00 UTC);

/// A SunSpec inverter that answers on base 40 000.
///
/// Publishes model 1 (common), 103 (inverter) and 123 (immediate controls), in
/// the chain layout a real device uses: `SunS`, then `(id, len, body…)` blocks,
/// then the `0xffff` sentinel.
struct Inverter {
    map: std::collections::BTreeMap<u16, u16>,
    curtailed_to: Option<u16>,
    /// A floor the device clips `WMaxLimPct` to, the way a real inverter with a
    /// minimum curtailment step does.
    clips_below: Option<u16>,
    /// Whether the device stores `WMaxLimPct` and leaves `WMaxLim_Ena` at zero.
    ignores_enable: bool,
    /// Every read it was asked for, as `(address, count)`.
    reads: Vec<(u16, u16)>,
}

impl Inverter {
    fn new() -> Self {
        let mut map = std::collections::BTreeMap::new();
        map.insert(40_000, 0x5375);
        map.insert(40_001, 0x6e53);

        // Model 1, common.
        map.insert(40_002, 1);
        map.insert(40_003, 66);
        for i in 0..66u16 {
            map.insert(40_004 + i, 0);
        }

        // Model 103 at 40 070.
        map.insert(40_070, 103);
        map.insert(40_071, 50);
        for i in 0..50u16 {
            map.insert(40_072 + i, 0);
        }
        // Model 103's real layout, which is the point of writing it out: A,
        // AphA, AphB, AphC, A_SF, PPVphAB, PPVphBC, PPVphCA, PhVphA, PhVphB,
        // PhVphC, V_SF, **W**, **W_SF**, **Hz**, **Hz_SF**, …
        map.insert(40_072 + 12, 231); // W
        map.insert(40_072 + 13, 1); // W_SF: ×10¹ = 2 310 W
        map.insert(40_072 + 14, 4998); // Hz
        map.insert(40_072 + 15, (-2i16) as u16); // Hz_SF: ×10⁻² = 49,98 Hz

        // The next header sits at `header + 2 + len`, which is the whole of how
        // a chain is walked: 40 070 + 2 + 50.
        map.insert(40_122, 123);
        map.insert(40_123, 24);
        for i in 0..24u16 {
            map.insert(40_124 + i, 0);
        }

        // …and the sentinel after that one: 40 122 + 2 + 24.
        map.insert(40_148, 0xffff);
        map.insert(40_149, 0);

        Self {
            map,
            curtailed_to: None,
            clips_below: None,
            ignores_enable: false,
            reads: Vec::new(),
        }
    }

    /// A device that stores `WMaxLimPct` with this `WMaxLimPct_SF`. The offset
    /// is the model definition's, as the `sunspec` crate publishes it.
    fn scaling_the_limit_by(mut self, scale: u16) -> Self {
        let at = sunspec::models::model123::Model123::W_MAX_LIM_PCT_SF.offset;
        self.map.insert(40_124 + at, scale);
        self
    }

    /// A device that also publishes model 701 after model 123, producing
    /// 3 000 W at half throttle. Its length and its points' offsets are the
    /// model definition's.
    fn with_der(mut self) -> Self {
        use sunspec::Group as _;
        use sunspec::models::model701::Model701;
        let (header, body) = (40_148, 40_150);
        self.map.insert(header, 701);
        self.map.insert(header + 1, Model701::LEN);
        for i in 0..Model701::LEN {
            self.map.insert(body + i, 0);
        }
        self.map.insert(body + Model701::W.offset, 300);
        self.map.insert(body + Model701::W_SF.offset, 1);
        self.map.insert(body + Model701::THROT_PCT.offset, 50);
        self.map.insert(body + Model701::LEN, 0xffff);
        self.map.insert(body + Model701::LEN + 1, 0);
        self
    }

    /// A device that will not go below `floor` per cent.
    fn clipping_below(mut self, floor: u16) -> Self {
        self.clips_below = Some(floor);
        self
    }

    /// A device that stores the setpoint and never switches it on.
    fn ignoring_the_enable_flag(mut self) -> Self {
        self.ignores_enable = true;
        self
    }

    /// Answer one request, the way a device would.
    fn answer(&mut self, bytes: &[u8]) -> Vec<u8> {
        let transaction = u16::from_be_bytes([bytes[0], bytes[1]]);
        let unit = bytes[6];
        let function = bytes[7];
        let address = u16::from_be_bytes([bytes[8], bytes[9]]);

        let pdu = match function {
            0x03 => {
                let count = u16::from_be_bytes([bytes[10], bytes[11]]);
                self.reads.push((address, count));
                // More than 125 registers cannot be described in one answer; a
                // real device refuses the request.
                if count > 125 {
                    return self.framed(transaction, unit, &[0x83, 0x03]);
                }
                // An unmapped address is `0x02`, illegal data address — which is
                // how a real device ends a model walk.
                if self.map.contains_key(&address) {
                    let mut pdu = vec![0x03, u8::try_from(count * 2).unwrap()];
                    for i in 0..count {
                        let v = self.map.get(&(address + i)).copied().unwrap_or(0);
                        pdu.extend_from_slice(&v.to_be_bytes());
                    }
                    pdu
                } else {
                    vec![0x83, 0x02]
                }
            }
            0x10 => {
                let count = u16::from_be_bytes([bytes[10], bytes[11]]);
                // Stored into the map, because a device that answers a write
                // and does not change its registers is a device no read-back
                // could ever catch out.
                for i in 0..count {
                    let at = usize::from(i) * 2;
                    let mut v = u16::from_be_bytes([bytes[13 + at], bytes[14 + at]]);
                    if i == 0 {
                        v = self.clips_below.map_or(v, |floor| v.max(floor));
                    }
                    if i == 4 && self.ignores_enable {
                        v = 0;
                    }
                    self.map.insert(address + i, v);
                }
                // Read where model 123 keeps `WMaxLimPct` — body + 3, after the
                // three `Conn` registers — and not wherever the write landed: a
                // device takes a limit from that register and no other.
                self.curtailed_to = (address == 40_124 + 3)
                    .then(|| self.map.get(&address).copied())
                    .flatten();
                let mut pdu = vec![0x10];
                pdu.extend_from_slice(&address.to_be_bytes());
                pdu.extend_from_slice(&count.to_be_bytes());
                pdu
            }
            _ => vec![function | 0x80, 0x01],
        };

        self.framed(transaction, unit, &pdu)
    }

    #[expect(clippy::unused_self, reason = "reads as the device framing its answer")]
    fn framed(&self, transaction: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&transaction.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&(u16::try_from(pdu.len() + 1).unwrap()).to_be_bytes());
        out.push(unit);
        out.extend_from_slice(pdu);
        out
    }
}

/// Run the driver against the device until it stops asking, or `limit` rounds.
fn exchange(driver: &mut impl Driver, device: &mut Inverter, now: OffsetDateTime, limit: usize) {
    for _ in 0..limit {
        let Some(out) = driver.poll_transmit() else {
            return;
        };
        let reply = device.answer(&out);
        driver.on_bytes(&reply, now).expect("a well-formed reply");
    }
}

fn driver() -> SunSpec {
    SunSpec::new(
        AssetId::new("wechselrichter").expect("a valid identifier"),
        1,
        Cadence::default(),
    )
}

#[test]
fn a_device_is_discovered_by_walking_its_own_chain() {
    // Where model 103 lives differs between manufacturers and between firmware
    // versions of the same one, so it is read off the device rather than
    // configured.
    let mut d = driver();
    let mut device = Inverter::new();
    assert_eq!(d.kind(), None, "nothing is known before the first byte");

    d.on_timeout(T0);
    exchange(&mut d, &mut device, T0, 60);

    assert!(d.models().has(1), "the common model");
    assert!(d.models().has(103), "the inverter model");
    assert!(d.models().has(123), "the curtailment control");
    assert!(!d.models().has(203), "and not a meter it does not have");
    assert_eq!(d.kind(), Some(Kind::Inverter));
}

#[test]
fn a_reading_comes_back_with_its_scale_factor_applied() {
    // The single most common way a SunSpec integration is wrong: `W = 231` with
    // `W_SF = 1` is 2 310 watts, not 231. A factor of ten still looks like a
    // plausible household, which is why it survives.
    let mut d = driver();
    let mut device = Inverter::new();
    d.on_timeout(T0);
    exchange(&mut d, &mut device, T0, 60);

    let measured = std::iter::from_fn(|| d.poll_event())
        .filter_map(|e| match e {
            DriverEvent::Measured(m) => Some(m),
            _ => None,
        })
        .last()
        .expect("a measurement");

    assert_eq!(
        measured.power,
        Some(Power::new(-2310.0)),
        "2 310 W, produced — negative in the load convention, because an \
         inverter is a source"
    );
    let hz = measured.frequency_hz.expect("a frequency");
    assert!((hz - 49.98).abs() < 0.01, "{hz}");
}

#[test]
fn an_inverter_with_no_model_701_does_not_claim_to_know_what_it_could_produce() {
    // The capability that cannot be worked around. A curtailed inverter asked
    // what it is producing answers with what the manager already commanded, so a
    // controller reading that alone never lifts its own curtailment. This device
    // publishes 103 and not 701, so it says it cannot tell — and the caller
    // falls back to a nameplate *knowing* that it has.
    let mut d = driver();
    let mut device = Inverter::new();
    d.on_timeout(T0);
    exchange(&mut d, &mut device, T0, 60);

    assert!(!d.models().reports_available_power());
    assert!(
        !d.capabilities().reports_available_power,
        "a driver may not claim a figure its protocol cannot give it"
    );
    assert!(d.capabilities().measures);
}

#[test]
fn a_device_that_stops_answering_makes_the_link_stale() {
    // The guard's whole defence against a silent device is that it stops
    // believing the last measurement. A driver that never said the link had gone
    // would leave a reading looking fresh because nothing contradicted it.
    let mut d = driver();
    let mut device = Inverter::new();
    d.on_timeout(T0);
    exchange(&mut d, &mut device, T0, 60);
    while d.poll_event().is_some() {}

    let later = T0 + Duration::seconds(2);
    d.on_timeout(later);
    assert!(d.poll_transmit().is_some(), "the next poll went out");

    d.on_timeout(later + Duration::seconds(30));
    let links: Vec<LinkState> = std::iter::from_fn(|| d.poll_event())
        .filter_map(|e| match e {
            DriverEvent::Link(l) => Some(l),
            _ => None,
        })
        .collect();
    assert!(
        links.contains(&LinkState::Stale),
        "silence has to be reported: {links:?}"
    );
}

#[test]
fn a_deadline_is_always_offered() {
    // A driver that returned `None` here would be woken only by bytes, and a
    // silent device sends none — so the timeout that notices the silence would
    // never fire.
    let mut d = driver();
    d.on_timeout(T0);
    assert!(d.poll_deadline().is_some(), "with a request outstanding");

    let mut device = Inverter::new();
    exchange(&mut d, &mut device, T0, 60);
    assert!(d.poll_deadline().is_some(), "and between polls");
}

#[test]
fn a_command_the_protocol_cannot_express_is_refused_rather_than_dropped() {
    // A command nobody sends and nobody reports is how a device quietly stops
    // being managed.
    let mut d = driver();
    let mut device = Inverter::new();
    d.on_timeout(T0);
    exchange(&mut d, &mut device, T0, 60);

    let err = d
        .command(&Command::ChargingCurrent(Current::new(16.0)), T0)
        .expect_err("an inverter has no charging current");
    assert!(matches!(err, DriverError::Unsupported(_)), "{err:?}");
}

#[test]
fn a_curtailment_needs_a_rating_because_the_model_is_a_percentage() {
    // Model 123 curtails in **per cent of WMax**, not in watts. A driver that
    // did not know the rating would have to invent one, and a percentage
    // computed from a guess is a curtailment wrong by whatever the guess was.
    let mut d = driver();
    let mut device = Inverter::new();
    d.on_timeout(T0);
    exchange(&mut d, &mut device, T0, 60);

    let err = d
        .command(&Command::ProductionCeiling(Power::from_kw(3.0)), T0)
        .expect_err("no rating was published, so no percentage can be computed");
    assert!(matches!(err, DriverError::Unsupported(_)), "{err:?}");
}

#[test]
fn a_curtailment_reaches_the_device_as_a_percentage_of_its_rating() {
    // The other half of the previous test, and the one that would have caught a
    // driver whose curtailment could never fire: nothing populated the rating,
    // so `with_rating` had no caller and every production ceiling was refused.
    // A control path that always refuses is indistinguishable from one that
    // works until the day a roof has to be turned down.
    let mut d = SunSpec::new(
        AssetId::new("wechselrichter").expect("a valid identifier"),
        1,
        Cadence::default(),
    )
    .with_rating(Power::from_kw(10.0));
    let mut device = Inverter::new();
    d.on_timeout(T0);
    exchange(&mut d, &mut device, T0, 60);
    while d.poll_event().is_some() {}

    d.command(&Command::ProductionCeiling(Power::from_kw(6.0)), T0)
        .expect("a ceiling on a device with a rating and a model 123");
    exchange(&mut d, &mut device, T0, 8);

    assert_eq!(
        device.curtailed_to,
        Some(60),
        "6 kW of a 10 kW inverter is sixty per cent, because model 123 curtails          in per cent of WMax and not in watts"
    );
    let outcome = command_outcome(&mut d).expect("the command is reported");
    assert!(outcome.accepted, "and the acknowledgement is reported");
    assert_eq!(
        outcome.confirmed,
        Some(Power::from_kw(6.0)),
        "read back off the device's own registers rather than assumed from what \
         was written: a Modbus write answer echoes the address and the count and \
         never the values"
    );
    assert_eq!(
        outcome.detail, None,
        "nothing to say about a device that obeyed"
    );
}

/// The first `CommandOutcome` the driver has to offer.
#[test]
fn a_ceiling_is_written_in_the_devices_own_scale() {
    // `WMaxLimPct_SF = −2` is what a Fronius or an SMA publishes: sixty per
    // cent is 6000 in the register. Written as 60 it is 0,6 % — a roof turned
    // off by a driver that believes it asked for a little less.
    let mut d = driver().with_rating(Power::from_kw(10.0));
    let mut device = Inverter::new().scaling_the_limit_by((-2i16).cast_unsigned());
    d.on_timeout(T0);
    exchange(&mut d, &mut device, T0, 60);
    while d.poll_event().is_some() {}

    d.command(&Command::ProductionCeiling(Power::from_kw(6.0)), T0)
        .expect("a ceiling on a device with a rating and a model 123");
    exchange(&mut d, &mut device, T0, 8);

    assert_eq!(device.curtailed_to, Some(6000));
    let outcome = command_outcome(&mut d).expect("the command is reported");
    assert!(outcome.accepted);
    assert_eq!(
        outcome.confirmed,
        Some(Power::from_kw(6.0)),
        "and read back through the same scale"
    );
    assert_eq!(outcome.detail, None);
}

#[test]
fn a_scale_factor_the_device_does_not_implement_is_no_scaling() {
    let mut d = driver().with_rating(Power::from_kw(10.0));
    let mut device = Inverter::new().scaling_the_limit_by(0x8000);
    d.on_timeout(T0);
    exchange(&mut d, &mut device, T0, 60);
    while d.poll_event().is_some() {}

    d.command(&Command::ProductionCeiling(Power::from_kw(6.0)), T0)
        .expect("a ceiling on a device with a rating and a model 123");
    exchange(&mut d, &mut device, T0, 8);
    assert_eq!(device.curtailed_to, Some(60));
}

#[test]
fn a_model_longer_than_one_read_is_fetched_in_pieces_and_decoded_whole() {
    // Model 701 is 153 registers and one Modbus read carries 125. Asked for in
    // one request it is refused or cut short, `ThrotPct` and `W_SF` never
    // arrive, and the device that was said to report what it could produce
    // reports nothing.
    let mut d = driver();
    let mut device = Inverter::new().with_der();
    d.on_timeout(T0);
    exchange(&mut d, &mut device, T0, 60);

    assert!(
        device.reads.iter().all(|&(_, count)| count <= 125),
        "no read may ask for more than the protocol can answer: {:?}",
        device.reads
    );
    assert!(
        device.reads.contains(&(40_150, 125)) && device.reads.contains(&(40_275, 28)),
        "125 registers, then the 28 that are left: {:?}",
        device.reads
    );
    let measured = std::iter::from_fn(|| d.poll_event())
        .filter_map(|e| match e {
            DriverEvent::Measured(m) => Some(m),
            _ => None,
        })
        .last()
        .expect("a measurement");
    assert!(measured.estimated_availability);
    assert_eq!(
        measured.available_power,
        Some(Power::new(6000.0)),
        "3 000 W at half throttle gives the control loop a 6 000 W estimate"
    );
}

fn command_outcome(d: &mut SunSpec) -> Option<ems_drv::CommandOutcome> {
    std::iter::from_fn(|| d.poll_event()).find_map(|e| match e {
        DriverEvent::Command(o) => Some(o),
        _ => None,
    })
}

/// A curtailed inverter that reads back where it was put, and the two ways it
/// does not.
fn curtailed(device: &mut Inverter) -> Option<ems_drv::CommandOutcome> {
    let mut d = SunSpec::new(
        AssetId::new("wechselrichter").expect("a valid identifier"),
        1,
        Cadence::default(),
    )
    .with_rating(Power::from_kw(10.0));
    d.on_timeout(T0);
    exchange(&mut d, device, T0, 60);
    while d.poll_event().is_some() {}

    d.command(&Command::ProductionCeiling(Power::from_kw(2.0)), T0)
        .expect("a ceiling on a device with a rating and a model 123");
    exchange(&mut d, device, T0, 8);
    command_outcome(&mut d)
}

#[test]
fn a_device_that_clips_the_setpoint_says_so_rather_than_agreeing() {
    // A real inverter with a minimum curtailment step takes 20 % and answers
    // the write exactly as one that obeyed — the Modbus write response echoes
    // the address and the quantity and never the values. The plan asked for
    // 2 kW and the roof will deliver 3, and every layer above would have gone
    // on believing 2 until the meter disagreed.
    let mut device = Inverter::new().clipping_below(30);
    let outcome = curtailed(&mut device).expect("the command is reported");

    assert!(
        outcome.accepted,
        "the limit is in force, so the command was obeyed — just not to the \
         value that was asked for"
    );
    assert_eq!(
        outcome.confirmed,
        Some(Power::from_kw(3.0)),
        "30 % of a 10 kW inverter, which is what the device actually holds"
    );
    assert!(
        outcome.detail.is_some_and(|d| d.contains("clipped")),
        "and it is named, because the difference between commanded and \
         confirmed is where a plan and a house stop agreeing"
    );
}

#[test]
fn a_setpoint_stored_but_never_switched_on_is_not_an_accepted_command() {
    // The classic way to curtail nothing at all and believe otherwise:
    // `WMaxLimPct` is written, `WMaxLim_Ena` stays at zero, and the device
    // acknowledges the write. Nothing about the answer distinguishes it from
    // obedience, and a § 14a Nachweis built on it would record a reduction that
    // never happened.
    let mut device = Inverter::new().ignoring_the_enable_flag();
    let outcome = curtailed(&mut device).expect("the command is reported");

    assert!(
        !outcome.accepted,
        "a limit that is not in force is not accepted"
    );
    assert!(
        outcome.detail.is_some_and(|d| d.contains("WMaxLim_Ena")),
        "and the reason is the register that says so"
    );
}

#[test]
fn a_delayed_write_is_confirmed_after_the_device_applies_it() {
    let cadence = Cadence::default();
    let applied = T0 + cadence.timeout - Duration::milliseconds(1);
    let mut driver = SunSpec::new(AssetId::new("inverter").unwrap(), 1, cadence)
        .with_rating(Power::from_kw(10.0));
    let mut inverter = Inverter::new();
    driver.on_timeout(T0);
    exchange(&mut driver, &mut inverter, T0, 60);
    while driver.poll_event().is_some() {}

    let requested = Power::from_kw(2.0);
    driver
        .command(&Command::ProductionCeiling(requested), T0)
        .unwrap();
    let write = driver.poll_transmit().expect("write request");
    driver.on_timeout(applied);
    assert!(command_outcome(&mut driver).is_none());
    driver.on_bytes(&inverter.answer(&write), applied).unwrap();
    exchange(&mut driver, &mut inverter, applied, 8);
    let outcome = command_outcome(&mut driver).expect("readback");
    assert!(outcome.accepted);
    assert_eq!(outcome.confirmed, Some(requested));
    assert_eq!(outcome.at, applied);
}

#[test]
fn a_command_the_device_never_answers_is_reported_as_a_failure() {
    // Silence at this seam is the dangerous case: nothing contradicts the
    // ceiling, so it reaches the evidence record as one that was issued and
    // obeyed. A command that was never answered was not obeyed, and the driver
    // is the only layer that knows.
    let mut d = SunSpec::new(
        AssetId::new("wechselrichter").expect("a valid identifier"),
        1,
        Cadence::default(),
    )
    .with_rating(Power::from_kw(10.0));
    let mut device = Inverter::new();
    d.on_timeout(T0);
    exchange(&mut d, &mut device, T0, 60);
    while d.poll_event().is_some() {}

    d.command(&Command::ProductionCeiling(Power::from_kw(6.0)), T0)
        .expect("a ceiling on a device with a rating and a model 123");
    let _ = d.poll_transmit().expect("a write went out");
    // …and nothing comes back.
    d.on_timeout(T0 + time::Duration::minutes(5));

    let outcome = command_outcome(&mut d).expect("the command is reported");
    assert!(!outcome.accepted);
    assert_eq!(outcome.confirmed, None);
}

#[test]
fn a_garbage_scale_factor_does_not_take_the_box_down() {
    // `10^exponent` with an exponent read off a wire is an infinity waiting to
    // happen, and `Power` rejects a value that is not finite — so one misaligned
    // register would panic the control loop. The input comes from a device
    // nobody in this workspace controls, so it has to be survivable.
    let mut d = driver();
    let mut device = Inverter::new();
    // W_SF far outside SunSpec's own −10..10.
    device.map.insert(40_072 + 13, 4998);
    d.on_timeout(T0);
    exchange(&mut d, &mut device, T0, 60);

    let measured = std::iter::from_fn(|| d.poll_event())
        .filter_map(|e| match e {
            DriverEvent::Measured(m) => Some(m),
            _ => None,
        })
        .last()
        .expect("a measurement, rather than a panic");
    let w = measured.power.expect("a power").get();
    assert!(w.is_finite(), "{w}");
}

#[test]
fn a_frame_split_across_reads_is_acted_on_only_once_it_is_whole() {
    // A TCP read is not a message boundary.
    let mut d = driver();
    let mut device = Inverter::new();
    d.on_timeout(T0);

    let first = d.poll_transmit().expect("a probe");
    let reply = device.answer(&first);
    for byte in &reply[..reply.len() - 1] {
        d.on_bytes(&[*byte], T0).expect("well formed");
        assert!(
            d.poll_transmit().is_none(),
            "a partial frame must not move the walk on"
        );
    }
    d.on_bytes(&[reply[reply.len() - 1]], T0)
        .expect("well formed");
    assert!(
        d.poll_transmit().is_some(),
        "and the whole one must, at once"
    );
}

#[test]
fn rubbish_on_the_wire_is_an_error_and_not_a_measurement() {
    let mut d = driver();
    let err = d
        .on_bytes(&[0, 1, 0, 7, 0, 3, 1, 0x03, 0x00], T0)
        .expect_err("protocol identifier 7 is not Modbus");
    assert!(matches!(err, DriverError::Malformed(_)), "{err:?}");
}

#[test]
fn the_frame_codec_and_the_device_agree() {
    // A guard against this test's own device being wrong: what the encoder
    // produces is what the decoder reads.
    let request = frame::Request {
        transaction: 42,
        unit: 1,
        body: RequestBody::Read {
            space: ems_drv::modbus::frame::Space::Holding,
            address: 40_000,
            count: 2,
        },
    };
    let mut device = Inverter::new();
    let reply = device.answer(&request.encode());
    let (response, used) = frame::decode(&reply).unwrap().expect("a whole frame");
    assert_eq!(used, reply.len());
    assert_eq!(response.transaction, 42);
    assert_eq!(
        response.body,
        ResponseBody::Registers(vec![0x5375, 0x6e53]),
        "the device says SunS"
    );
}

#[test]
fn a_read_only_inverter_refuses_the_write_at_the_driver_boundary() {
    let mut d = driver().with_rating(Power::from_kw(10.0)).listening_only();
    let mut device = Inverter::new();
    d.on_link(LinkState::Up, T0);
    exchange(&mut d, &mut device, T0, 60);
    assert_eq!(d.kind(), Some(Kind::Inverter));
    assert!(matches!(
        d.command(&Command::ProductionCeiling(Power::ZERO), T0),
        Err(DriverError::Unsupported(_))
    ));
    assert!(d.poll_transmit().is_none());
}

#[test]
fn a_new_walk_invalidates_the_previous_control_registers() {
    let mut d = driver().with_rating(Power::from_kw(10.0));
    let mut device = Inverter::new();
    d.on_link(LinkState::Up, T0);
    exchange(&mut d, &mut device, T0, 60);
    assert!(d.can_limit_production());

    // Replace the unit with one advertising only a prefix of model 123.
    device.map.insert(40_123, 2);
    device.map.insert(40_126, 0xffff);
    let ceiling = Command::ProductionCeiling(Power::from_kw(4.0));
    for state in [LinkState::Down, LinkState::Up] {
        d.on_link(state, T0);
        assert_eq!(d.kind(), None);
        assert!(!d.can_limit_production());
        assert!(matches!(
            d.command(&ceiling, T0),
            Err(DriverError::NotConnected)
        ));
    }
    exchange(&mut d, &mut device, T0, 60);
    assert!(d.models().has(123));
    assert!(!d.can_limit_production());
    assert!(matches!(
        d.command(&ceiling, T0),
        Err(DriverError::Unsupported(_))
    ));
    assert!(d.poll_transmit().is_none());
}

#[test]
fn a_write_echo_must_name_the_requested_registers() {
    for (address, count) in [(40_128_u16, 5_u16), (40_127, 4)] {
        let mut d = driver().with_rating(Power::from_kw(10.0));
        let mut device = Inverter::new();
        d.on_link(LinkState::Up, T0);
        exchange(&mut d, &mut device, T0, 60);
        d.command(&Command::ProductionCeiling(Power::from_kw(6.0)), T0)
            .unwrap();
        let scale = d.poll_transmit().unwrap();
        d.on_bytes(&device.answer(&scale), T0).unwrap();
        let write = d.poll_transmit().unwrap();
        let mut echo = device.answer(&write);
        echo[8..10].copy_from_slice(&address.to_be_bytes());
        echo[10..12].copy_from_slice(&count.to_be_bytes());
        d.on_bytes(&echo, T0).unwrap();
        let outcome = command_outcome(&mut d).unwrap();
        assert!(!outcome.accepted && outcome.confirmed.is_none());
        assert!(d.poll_transmit().is_none());
    }
}

#[test]
fn each_integer_meter_model_reports_both_energy_registers() {
    use sunspec::Group as _;
    use sunspec::models::model203::Model203;
    for model in 201..=204 {
        let mut device = Inverter::new();
        let header = 40_070;
        let body = header + 2;
        device.map.insert(header, model);
        device.map.insert(header + 1, Model203::LEN);
        for offset in 0..Model203::LEN {
            device.map.insert(body + offset, 0);
        }
        device
            .map
            .insert(body + Model203::TOT_WH_IMP.offset + 1, 2137);
        device
            .map
            .insert(body + Model203::TOT_WH_EXP.offset + 1, 134);
        device.map.insert(body + Model203::LEN, 0xffff);
        device.map.insert(body + Model203::LEN + 1, 0);
        let mut d = driver().listening_only();
        d.on_link(LinkState::Up, T0);
        exchange(&mut d, &mut device, T0, 60);
        let m = std::iter::from_fn(|| d.poll_event())
            .find_map(|event| match event {
                DriverEvent::Measured(m) => Some(m),
                _ => None,
            })
            .expect("meter telemetry");
        assert_eq!(d.kind(), Some(Kind::Meter));
        assert_eq!(m.energy_in.unwrap().get(), 2137.0);
        assert_eq!(m.energy_out.unwrap().get(), 134.0);
    }
}

#[test]
fn commissioned_meter_reports_identity_and_rechecks_the_profile_after_reconnect() {
    use ems_drv::modbus::Meter;
    use sunspec::Group as _;
    use sunspec::models::model203::Model203;

    let mut peer = Inverter::new();
    peer.map.insert(40_004, u16::from_be_bytes(*b"XY"));
    peer.map.insert(40_070, 203);
    peer.map.insert(40_071, Model203::LEN);
    let body = 40_072;
    for offset in 0..Model203::LEN {
        peer.map.insert(body + offset, 0);
    }
    peer.map
        .insert(body + Model203::W.offset, (-30_i16).cast_unsigned());
    peer.map.insert(body + Model203::TOT_WH_IMP.offset + 1, 521);
    peer.map.insert(body + Model203::TOT_WH_EXP.offset + 1, 281);
    peer.map.insert(body + Model203::LEN, 0xffff);
    peer.map.insert(body + Model203::LEN + 1, 0);

    let mut meter = Meter::new(AssetId::new("grid-meter").unwrap(), 3, Cadence::default());
    meter.on_link(LinkState::Connecting, T0);
    exchange(&mut meter, &mut peer, T0, 20);
    let identity = meter.meter_identity().unwrap();
    assert_eq!(identity.manufacturer, "XY");
    assert!(identity.connection_meter);
    let mut events = std::iter::from_fn(|| meter.poll_event());
    assert!(matches!(
        events.next(),
        Some(DriverEvent::Link(LinkState::Connecting))
    ));
    assert!(matches!(
        events.next(),
        Some(DriverEvent::Link(LinkState::Up))
    ));
    let Some(DriverEvent::Measured(reading)) = events.next() else {
        panic!("meter sample")
    };
    assert_eq!(reading.power.unwrap().get(), -30.);
    assert_eq!(reading.energy_in.unwrap().get(), 521.);
    assert_eq!(reading.energy_out.unwrap().get(), 281.);
    assert!(events.next().is_none());

    let mut tcp_meter = driver().listening_only();
    tcp_meter.on_link(LinkState::Up, T0);
    exchange(&mut tcp_meter, &mut peer, T0, 20);
    let tcp_reading = std::iter::from_fn(|| tcp_meter.poll_event())
        .find_map(|event| {
            if let DriverEvent::Measured(m) = event {
                Some(m)
            } else {
                None
            }
        })
        .expect("TCP meter sample from the same registers");
    assert_eq!(reading, tcp_reading);
    assert!(
        meter
            .command(&Command::ProductionCeiling(Power::from_kw(1.)), T0)
            .is_err()
    );

    let end = T0 + Duration::seconds(1);
    meter.on_timeout(end);
    peer.map.insert(body + Model203::W_SF.offset, 0x8000);
    let request = meter.poll_transmit().unwrap();
    assert!(meter.on_bytes(&peer.answer(&request), end).is_err());
    assert!(matches!(
        meter.poll_event(),
        Some(DriverEvent::Link(LinkState::Stale))
    ));
    assert!(meter.poll_event().is_none());
    assert!(meter.meter_identity().is_none());

    meter.on_timeout(end + Duration::seconds(1));
    let mut other = Inverter::new();
    exchange(&mut meter, &mut other, end + Duration::seconds(1), 20);
    assert!(meter.meter_identity().is_none());
    assert!(
        meter.poll_event().is_none(),
        "a replacement inverter cannot qualify as a grid meter"
    );
    assert!(meter.poll_transmit().is_none());
}
