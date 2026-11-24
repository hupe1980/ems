//! Received OCPP evidence from two synthetic charging stations.
use ems_drv::ocpp::{
    ChargingProfileStatus, DateTime, MeterValuesRequest, SetChargingProfileResponse,
    station_import_kw,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod wire;

pub(super) const BANK_LIMIT_KW: i32 = 24;

#[derive(Clone, Deserialize)]
struct Received<T> {
    received_at: DateTime,
    payload: T,
}

#[derive(Deserialize)]
struct StationScript {
    profile_reply_at: DateTime,
    initial: Received<Vec<MeterValuesRequest>>,
    demand_at_deadline: Received<Vec<MeterValuesRequest>>,
    stale: Received<Vec<MeterValuesRequest>>,
    delayed_receipt_at: Option<DateTime>,
    after_rejoin: Received<Vec<MeterValuesRequest>>,
}

#[derive(Deserialize)]
pub(super) struct Timeline {
    pub requested_at: DateTime,
    pub first_sample_at: DateTime,
    pub deadline_at: DateTime,
    pub handover_at: DateTime,
    pub rejoined_at: DateTime,
    pub buffered_at: DateTime,
    pub restored_at: DateTime,
}

#[derive(Deserialize)]
struct Script {
    timeline: Timeline,
    west_01: StationScript,
    west_02: StationScript,
}

struct Trace {
    profile_response: Received<SetChargingProfileResponse>,
    initial: Received<Vec<MeterValuesRequest>>,
    finals: BTreeMap<Outcome, Received<Vec<MeterValuesRequest>>>,
}

pub(super) struct Recording {
    pub timeline: Timeline,
    traces: [Trace; 2],
    buffered: Received<Vec<MeterValuesRequest>>,
    fresh: [Received<Vec<MeterValuesRequest>>; 2],
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Outcome {
    Confirmed,
    Above,
    Missing,
    Stale,
    Delayed,
    Zero,
    Rejoin,
}

#[derive(Serialize)]
pub(super) struct Station {
    pub id: &'static str,
    pub profile_accepted: Option<bool>,
    pub connected: bool,
    #[serde(serialize_with = "Option::<DateTime>::serialize")]
    pub buffered_received_at: Option<DateTime>,
    #[serde(serialize_with = "Option::<DateTime>::serialize")]
    pub final_measured_at: Option<DateTime>,
    pub initial_kw: Option<f64>,
    pub final_kw: Option<f64>,
    #[serde(serialize_with = "Option::<DateTime>::serialize")]
    pub final_received_at: Option<DateTime>,
}

impl Recording {
    pub async fn load() -> anyhow::Result<Self> {
        Self::simulate(include_str!("../examples/charging-bank.json")).await
    }

    pub(super) async fn simulate(script: &str) -> anyhow::Result<Self> {
        wire::run(serde_json::from_str(script)?, BANK_LIMIT_KW).await
    }

    pub(super) async fn measure_bank(bank_kw: i32) -> anyhow::Result<(DateTime, Option<f64>)> {
        anyhow::ensure!(
            (0..=BANK_LIMIT_KW).contains(&bank_kw),
            "charging budget exceeded"
        );
        let script = serde_json::from_str(include_str!("../examples/charging-bank.json"))?;
        let recording = wire::run(script, bank_kw).await?;
        let measured = recording
            .stations(Outcome::Confirmed, recording.timeline.deadline_at)
            .iter()
            .map(|station| station.final_kw)
            .sum();
        Ok((recording.timeline.deadline_at, measured))
    }

    pub fn stations(&self, outcome: Outcome, at: DateTime) -> [Station; 2] {
        [0, 1].map(|index| {
            let trace = &self.traces[index];
            let west2 = index == 1;
            let rejoin = outcome == Outcome::Rejoin;
            let connected = !(rejoin
                && west2
                && at >= self.timeline.deadline_at
                && at < self.timeline.rejoined_at);
            let fresh_due = rejoin && at >= self.timeline.restored_at;
            let measured_at = if fresh_due {
                self.timeline.restored_at
            } else {
                self.timeline.deadline_at
            };
            let event = if fresh_due {
                Some(&self.fresh[index])
            } else if rejoin && !west2 {
                trace.finals.get(&Outcome::Confirmed)
            } else {
                trace.finals.get(&outcome)
            };
            let final_evidence = event
                .filter(|event| event.received_at <= at && measured_at <= event.received_at)
                .and_then(|event| {
                    station_import_kw(&event.payload, measured_at)
                        .map(|power| (power, event.received_at))
                });
            let buffered_received_at = (rejoin
                && west2
                && self.buffered.received_at <= at
                && station_import_kw(&self.buffered.payload, self.timeline.first_sample_at)
                    .is_some())
            .then_some(self.buffered.received_at);
            Station {
                id: if west2 { "West-02" } else { "West-01" },
                connected,
                final_measured_at: final_evidence.map(|_| measured_at),
                buffered_received_at,
                profile_accepted: (trace.profile_response.received_at <= at).then(|| {
                    trace.profile_response.payload.status == ChargingProfileStatus::Accepted
                }),
                initial_kw: (trace.initial.received_at <= at)
                    .then(|| {
                        station_import_kw(&trace.initial.payload, self.timeline.first_sample_at)
                    })
                    .flatten(),
                final_kw: final_evidence.map(|(kw, _)| kw),
                final_received_at: final_evidence.map(|(_, arrival)| arrival),
            }
        })
    }
}
