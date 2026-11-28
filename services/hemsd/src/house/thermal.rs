//! Independent winter appliance and household controller on a loopback S2 WebSocket.

use crate::scenario::service::{ROOM_TOLERANCE_MK, WATER_TOLERANCE_WH};
use anyhow::{Context, Result, ensure};
use ems_core::prelude::{Asset, Energy, Measurement, Power};
use ems_flex::{
    Instructed, Offer, Ratings, Session, SessionEvent, describe_dhw, resource_manager_details,
};
use ems_sim::{BuildingSim, TankSim};
use futures_util::{SinkExt, StreamExt};
use s2_kit::prelude::{CemConfig, CemEvent, CemSession, ControlType, Id, frbc};
use s2_kit::types::common::InstructionStatus;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::io::Write as _;
use std::net::{Ipv4Addr, SocketAddr};
use std::process::Stdio;
use time::{Duration, OffsetDateTime};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio_tungstenite::{accept_async, connect_async, tungstenite::Message};

const MINUTES: u8 = 15;
const LEASE: u8 = 5;
const BAND_MK: [i32; 2] = [293_150, 295_150];
const INITIAL_ROOM_C: f64 = 21.0;
const DRAW_WH: f64 = 600.0;
const OUTDOOR_C: f64 = -3.0;
pub(super) const START: OffsetDateTime = time::macros::datetime!(2026-01-15 06:00 UTC);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Observation {
    instruction: Id,
    minute: u8,
    room_mk: f64,
    stored_wh: f64,
    deficit_wh: f64,
    heater_w: f64,
}

#[derive(Serialize)]
pub(super) struct Arm {
    instructions: Vec<Id>,
    requested_factor: f64,
    imported_wh: f64,
    exported_wh: f64,
    #[serde(with = "time::serde::rfc3339")]
    received_at: OffsetDateTime,
    observations: Vec<Observation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    departure: Option<super::departure::Receipt>,
    room: Outcome,
    water: Outcome,
    pub(super) closing_shortfall_wh: Option<f64>,
}

impl std::fmt::Display for Arm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Room · {}. Hot water · {}",
            self.room.label(),
            self.water.label()
        )?;
        if let Some(wh) = self
            .closing_shortfall_wh
            .filter(|wh| *wh > WATER_TOLERANCE_WH)
        {
            write!(f, " ({wh:.0} Wh below target)")?;
        }
        if let Some(receipt) = &self.departure {
            write!(f, "\n  {receipt}")?;
        }
        Ok(())
    }
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase", tag = "state")]
enum Outcome {
    Met,
    Short { minute: u8 },
    Unknown { minute: u8 },
}

impl Outcome {
    fn minute(&self) -> Option<u8> {
        match self {
            Self::Met => None,
            Self::Short { minute } | Self::Unknown { minute } => Some(*minute),
        }
    }
    fn label(&self) -> String {
        match self {
            Self::Met => "Met".into(),
            Self::Short { minute } => format!("Short at minute {minute}"),
            Self::Unknown { minute } => format!("Unknown at minute {minute}"),
        }
    }
}

/// Both decisions and their independently recorded measurements.
#[derive(Serialize)]
pub struct Receipt {
    #[serde(with = "time::serde::rfc3339")]
    start: OffsetDateTime,
    duration_minutes: u8,
    equivalent_thermal_service: bool,
    target_wh: u32,
    fixture_sha256: String,
    occupied_band_mk: [i32; 2],
    lease_minutes: u8,
    managed: Arm,
    baseline: Arm,
}

/// Run the resource manager in a disposable process. Stdout carries its sensor log.
///
/// # Errors
/// Returns a socket, protocol or fixture configuration error.
pub async fn peer(missing_close: bool) -> Result<()> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    println!("{}", listener.local_addr()?);
    std::io::stdout().flush()?;
    let (tcp, _) = listener.accept().await?;
    let mut socket = accept_async(tcp).await?;
    let house = crate::site::Household::build(&crate::HouseholdConfig::default())?;
    let asset = house
        .site
        .assets
        .iter()
        .find(|a| matches!(a, Asset::Dhw(_)))
        .context("fixture needs a hot-water tank")?;
    let Asset::Dhw(tank) = asset else {
        anyhow::bail!("expected hot-water tank")
    };
    let mut plant = TankSim::new(tank.usable_heat(), tank.heater);
    plant.cop = tank.cop;
    plant.standing_loss = tank.standing_loss;
    let mut room = BuildingSim::new(INITIAL_ROOM_C);
    let details = resource_manager_details(asset, asset.meta().phases.default_mode(), false);
    let mut rm = Session::new(
        asset.id().clone(),
        details,
        Offer::Dhw(Box::new(describe_dhw(tank, START))),
        Ratings::draws(tank.heater),
    );
    rm.open();
    let mut minute = 0;
    let mut deficit = 0.0;

    let mut last_instruction = None;
    loop {
        while let Some(m) = rm.poll_transmit() {
            socket
                .send(Message::text(serde_json::to_string(&m)?))
                .await?;
        }
        if minute == MINUTES {
            break;
        }
        let at = START + Duration::minutes(i64::from(minute));
        let frame = socket
            .next()
            .await
            .context("CEM disconnected without closing")??;
        let (instruction, watts, end) = if frame.is_close() {
            (
                last_instruction.context("closed before first instruction")?,
                Power::ZERO,
                MINUTES,
            )
        } else {
            let m = serde_json::from_str(frame.to_text()?)?;
            rm.on_message(&m, at);
            let Some((instruction, watts)) = rm.drain().into_iter().find_map(|event| match event {
                SessionEvent::Instructed {
                    wanted: Instructed::Power(watts),
                    instruction,
                    ..
                } => Some((instruction, watts)),
                _ => None,
            }) else {
                continue;
            };
            last_instruction = Some(instruction);
            (instruction, watts, (minute + LEASE).min(MINUTES))
        };
        let mut sampled_power = Vec::new();
        let mut samples = Vec::new();
        while minute < end {
            minute += 1;
            let draw = if minute == MINUTES {
                Energy::new(DRAW_WH)
            } else {
                Energy::ZERO
            };
            let (measured, short) = plant.step(watts, draw, Duration::minutes(1));
            sampled_power.push(measured.get());
            deficit += short.get();
            room.step(Power::ZERO, OUTDOOR_C, 0.0, Duration::minutes(1));
            if !missing_close || minute != MINUTES {
                samples.push(Observation {
                    instruction,
                    minute,
                    room_mk: ((room.state.indoor_c + 273.15) * 1000.0).round(),
                    stored_wh: plant.stored.get(),
                    deficit_wh: deficit,
                    heater_w: measured.get(),
                });
            }
        }
        println!("{}", serde_json::to_string(&(sampled_power, samples))?);
    }
    Ok(())
}

fn instruct(
    home: &crate::site::Household,
    cem: &mut CemSession,
    factor: f64,
    minute: u8,
    grid: Measurement,
) -> Result<Id> {
    let at = START + Duration::minutes(i64::from(minute));
    let id = home
        .dhw
        .as_ref()
        .context("missing household water heater")?;
    let rated = home
        .site
        .assets
        .iter()
        .find_map(|asset| match asset {
            Asset::Dhw(tank) if tank.meta.id == *id => Some(tank.heater),
            _ => None,
        })
        .context("missing commissioned tank")?;
    let granted = super::allowed_power(home, id, at, rated * factor, grid)?;
    let factor = (granted.get() / rated.get()).clamp(0.0, 1.0);
    let system = cem
        .registry()
        .frbc
        .as_ref()
        .context("missing FRBC description")?;
    let actuator = system.actuators.first().context("missing actuator")?;
    let mode = actuator
        .operation_modes
        .first()
        .context("missing heating mode")?;
    let instruction = Id::generate();
    let request = frbc::Instruction::builder()
        .id(instruction)
        .actuator_id(actuator.id)
        .operation_mode(mode.id)
        .operation_mode_factor(factor)
        .execution_time(at.into())
        .abnormal_condition(false)
        .build();
    cem.instruct(request, at.into())?;
    Ok(instruction)
}

async fn observe_window(
    lines: &mut SensorLog,
    grid: &mut super::grid::Grid,
    observations: &mut Vec<Observation>,
    minutes: u8,
) -> Result<Measurement> {
    let line = lines
        .next_line()
        .await?
        .context("missing appliance window")?;
    let (power, samples): (Vec<f64>, Vec<Observation>) = serde_json::from_str(&line)?;
    ensure!(
        power.len() == usize::from(minutes),
        "incomplete appliance power window"
    );
    for watts in power {
        ensure!(watts.is_finite() && watts >= 0.0, "invalid appliance power");
        grid.advance(Power::new(watts), 60)?;
    }
    observations.extend(samples);
    grid.sample().await
}

type SensorLog = tokio::io::Lines<BufReader<tokio::process::ChildStdout>>;
type ControllerSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect_peer(
    missing: bool,
) -> Result<(tokio::process::Child, SensorLog, ControllerSocket)> {
    let mut command = Command::new(std::env::current_exe()?);
    command.arg("thermal-peer");
    if missing {
        command.arg("--missing-close");
    }
    let mut child = command.kill_on_drop(true).stdout(Stdio::piped()).spawn()?;
    let mut lines = BufReader::new(child.stdout.take().context("appliance output")?).lines();
    let address: SocketAddr = lines
        .next_line()
        .await?
        .context("appliance address")?
        .parse()?;
    ensure!(address.ip().is_loopback(), "expected local appliance");
    let (socket, _) = connect_async(format!("ws://{address}")).await?;
    Ok((child, lines, socket))
}

async fn arm(
    factor: f64,
    target: u32,
    missing: bool,
    disconnect: bool,
    departure_wh: Option<i32>,
) -> Result<Arm> {
    let mut grid = super::grid::Grid::start(START).await?;
    let open = grid.sample().await?;
    arm_on(
        factor,
        target,
        missing,
        disconnect,
        departure_wh,
        &mut grid,
        open,
    )
    .await
}

pub(super) async fn arm_on(
    factor: f64,
    target: u32,
    missing: bool,
    disconnect: bool,
    departure_wh: Option<i32>,
    grid: &mut super::grid::Grid,
    open: Measurement,
) -> Result<Arm> {
    ensure!(
        open.at == START,
        "thermal opening outside the winter fixture"
    );
    let home = crate::site::Household::build(&crate::HouseholdConfig::default())?;
    let (mut child, mut lines, mut socket) = connect_peer(missing).await?;
    let mut measured = open;
    let mut observations = Vec::new();
    let now = START.into();
    let mut cem = CemSession::new(CemConfig::default());
    cem.open(now);
    let mut instructions = Vec::new();
    let mut minute = 0;
    'wire: loop {
        while let Some(out) = cem.poll_transmit() {
            socket.send(Message::text(out.text)).await?;
        }
        let frame = socket
            .next()
            .await
            .context("resource disconnected before acceptance")??;
        cem.handle_text(
            frame.to_text()?,
            (START + Duration::minutes(i64::from(minute))).into(),
        );
        while let Some(event) = cem.poll_event() {
            match event {
                CemEvent::ResourceDescribed(_) => {
                    cem.select_control_type(ControlType::FillRateBasedControl, now)?;
                }
                CemEvent::Description { .. }
                    if instructions.is_empty() && cem.registry().frbc.is_some() =>
                {
                    instructions.push(instruct(&home, &mut cem, factor, minute, measured)?);
                }
                CemEvent::InstructionStatus(status)
                    if instructions.last() == Some(&status.instruction_id)
                        && status.status_type == InstructionStatus::Accepted =>
                {
                    measured = observe_window(&mut lines, grid, &mut observations, LEASE).await?;
                    minute += LEASE;
                    if minute == MINUTES {
                        break 'wire;
                    }
                    if disconnect {
                        socket.close(None).await?;
                        measured =
                            observe_window(&mut lines, grid, &mut observations, MINUTES - minute)
                                .await?;
                        break 'wire;
                    }
                    instructions.push(instruct(&home, &mut cem, factor, minute, measured)?);
                }
                CemEvent::Nacked { status, .. } => {
                    anyhow::bail!("resource declined message: {status:?}")
                }
                _ => {}
            }
        }
    }
    let received_at = OffsetDateTime::now_utc();
    ensure!(child.wait().await?.success(), "appliance failed");
    let (room, water) = assess(&observations, &instructions, target);
    let closing_shortfall_wh = reading_for(&observations, instructions.last(), MINUTES)
        .map(|s| (f64::from(target) - s.stored_wh).max(0.0));
    let departure = match departure_wh {
        Some(goal) => {
            Some(Box::pin(super::departure::rehearse_on(goal, false, grid, measured)).await?)
        }
        None => None,
    };
    Ok(Arm {
        departure,
        closing_shortfall_wh,
        instructions,
        requested_factor: factor,
        imported_wh: (measured.energy_in.context("missing thermal close import")?
            - open.energy_in.context("missing thermal opening import")?)
        .get(),
        exported_wh: (measured
            .energy_out
            .context("missing thermal close export")?
            - open.energy_out.context("missing thermal opening export")?)
        .get(),
        received_at,
        observations,
        room,
        water,
    })
}

fn reading_for<'a>(
    samples: &'a [Observation],
    expected: Option<&Id>,
    minute: u8,
) -> Option<&'a Observation> {
    let mut readings = samples
        .iter()
        .filter(|s| Some(&s.instruction) == expected && s.minute == minute);
    let first = readings.next()?;
    readings.all(|s| s == first).then_some(first)
}

fn assess(samples: &[Observation], instructions: &[Id], target: u32) -> (Outcome, Outcome) {
    let mut room = Outcome::Met;
    let mut water = Outcome::Met;
    for minute in 1..=MINUTES {
        let expected = instructions
            .get(usize::from((minute - 1) / LEASE))
            .or_else(|| instructions.last());
        let Some(s) = reading_for(samples, expected, minute) else {
            if room == Outcome::Met {
                room = Outcome::Unknown { minute };
            }
            if water == Outcome::Met {
                water = Outcome::Unknown { minute };
            }
            continue;
        };
        if !matches!(room, Outcome::Short { .. })
            && !(f64::from(BAND_MK[0] - ROOM_TOLERANCE_MK)
                ..=f64::from(BAND_MK[1] + ROOM_TOLERANCE_MK))
                .contains(&s.room_mk)
        {
            room = Outcome::Short { minute };
        }
        if !matches!(water, Outcome::Short { .. })
            && (s.deficit_wh > WATER_TOLERANCE_WH
                || (minute == MINUTES && f64::from(target) - s.stored_wh > WATER_TOLERANCE_WH))
        {
            water = Outcome::Short { minute };
        }
    }
    (room, water)
}

impl Arm {
    pub(super) fn met_service(&self) -> bool {
        self.room == Outcome::Met && self.water == Outcome::Met
    }

    pub(super) fn write_html(&self, html: &mut String) {
        let _ = write!(
            html,
            "<p>Requested heater output · {:.0}%</p>",
            self.requested_factor * 100.0
        );
        let mut outcomes = [("Room comfort", &self.room), ("Hot water", &self.water)];
        outcomes.sort_by_key(|(_, outcome)| outcome.minute().unwrap_or(u8::MAX));
        for (service, outcome) in outcomes {
            let _ = write!(html, "<p>{service} · <strong>{}</strong>", outcome.label());
            if service == "Hot water"
                && let Some(shortfall) = self.closing_shortfall_wh
                && shortfall > WATER_TOLERANCE_WH
            {
                let _ = write!(html, " · {shortfall:.2} Wh below target");
            }
            html.push_str("</p>");
        }
        let _ = write!(
            html,
            "<p>06:00–06:15 UTC · {:.0} Wh imported, {:.0} Wh exported</p>",
            self.imported_wh, self.exported_wh
        );
        html.push_str("<details><summary>Minute readings</summary><table><thead><tr><th>Minute</th><th>Room °C</th><th>Stored Wh</th><th>Unserved Wh</th></tr></thead><tbody>");
        for s in &self.observations {
            let _ = write!(
                html,
                "<tr><td>{}</td><td>{:.3}</td><td>{:.2}</td><td>{:.2}</td></tr>",
                s.minute,
                s.room_mk / 1000.0 - 273.15,
                s.stored_wh,
                s.deficit_wh
            );
        }
        html.push_str("</tbody></table></details>");
        if let Some(departure) = &self.departure {
            let _ = write!(
                html,
                "<details><summary>Following EV departure · 06:15–07:00 UTC</summary><p style='white-space:pre-line'>{departure}</p></details>"
            );
        }
    }
}

impl Receipt {
    /// A standalone keyboard-readable report of the same receipt returned as JSON.
    pub fn html(&self) -> String {
        let mut html = String::from(include_str!("../../ui/thermal-header.html"));
        let _ = write!(
            html,
            "<p>06:00–06:15 UTC · Room target 20–22 °C · Hot water target {} Wh at 06:15.</p>",
            self.target_wh
        );
        for (name, arm) in [("Managed", &self.managed), ("Baseline", &self.baseline)] {
            let _ = write!(html, "<section><h2>{name}</h2>");
            arm.write_html(&mut html);
            html.push_str("</section>");
        }
        html.push_str("</main></html>");
        html
    }
}

pub(super) fn fixture_digest(target: u32, departure_wh: Option<i32>) -> Result<String> {
    let house = crate::site::Household::build(&crate::HouseholdConfig::default())?;
    let tank = house
        .site
        .assets
        .iter()
        .find(|a| matches!(a, Asset::Dhw(_)))
        .context("fixture needs tank")?;
    let frozen = serde_json::to_vec(&(
        "winter-thermal-v1",
        START.unix_timestamp(),
        MINUTES,
        LEASE,
        BAND_MK,
        INITIAL_ROOM_C,
        OUTDOOR_C,
        DRAW_WH,
        target,
        departure_wh,
        tank,
    ))?;
    Ok(hex::encode(aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA256,
        &frozen,
    )))
}

/// Run matched 15-minute fixture arms. A resource timeout cannot leave a child running.
///
/// # Errors
/// Returns an error if either peer cannot be started or its evidence is unavailable.
pub async fn trial(
    target: u32,
    missing: bool,
    disconnect: bool,
    departure_wh: Option<i32>,
) -> Result<Receipt> {
    let fixture_sha256 = fixture_digest(target, departure_wh)?;
    let (managed, baseline) = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        tokio::try_join!(
            Box::pin(arm(1.0, target, missing, disconnect, departure_wh)),
            arm(0.5, target, false, false, departure_wh)
        )
    })
    .await
    .context("thermal rehearsal timed out")??;
    Ok(Receipt {
        start: START,
        duration_minutes: MINUTES,
        equivalent_thermal_service: [&managed, &baseline].iter().all(|a| a.met_service()),
        target_wh: target,
        occupied_band_mk: BAND_MK,
        lease_minutes: LEASE,
        fixture_sha256,
        managed,
        baseline,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_foreign_future_and_conflicting_minutes_preserve_service_boundaries() {
        let id = Id::generate();
        let mut samples = (1..=MINUTES)
            .map(|minute| Observation {
                instruction: id,
                minute,
                room_mk: 293_050.0,
                stored_wh: 1799.0,
                deficit_wh: 1.0,
                heater_w: 0.0,
            })
            .collect::<Vec<_>>();
        samples.push(samples[0].clone());
        assert_eq!(assess(&samples, &[id], 1800), (Outcome::Met, Outcome::Met));
        samples[0].room_mk -= 1.0;
        assert_eq!(
            assess(&samples, &[id], 1800),
            (
                Outcome::Unknown { minute: 1 },
                Outcome::Unknown { minute: 1 }
            )
        );
        samples.pop();
        let mut conflicting_close = samples.last().unwrap().clone();
        conflicting_close.stored_wh += 2.0;
        samples.push(conflicting_close);
        assert!(reading_for(&samples, Some(&id), MINUTES).is_none());
        samples.pop();
        samples[1].instruction = Id::generate();
        samples[2].minute = 16;
        samples[4].deficit_wh = 2.0;
        assert_eq!(
            assess(&samples, &[id], 1800),
            (Outcome::Short { minute: 1 }, Outcome::Short { minute: 5 })
        );
    }
}
