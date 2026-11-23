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
}

#[derive(Deserialize)]
pub(super) struct Timeline {
    pub requested_at: DateTime,
    pub first_sample_at: DateTime,
    pub deadline_at: DateTime,
    pub handover_at: DateTime,
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
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Outcome {
    Confirmed,
    Above,
    Missing,
    Stale,
    Delayed,
    Zero,
}

#[derive(Serialize)]
pub(super) struct Station {
    pub id: &'static str,
    pub profile_accepted: Option<bool>,
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
        wire::run(serde_json::from_str(script)?).await
    }

    pub fn stations(&self, outcome: Outcome, at: DateTime) -> [Station; 2] {
        [("West-01", &self.traces[0]), ("West-02", &self.traces[1])].map(|(id, trace)| {
            let final_evidence = trace.finals.get(&outcome).and_then(|event| {
                let arrival = event.received_at;
                if arrival > at {
                    return None;
                }
                station_import_kw(&event.payload, self.timeline.deadline_at)
                    .map(|power| (power, arrival))
            });
            Station {
                id,
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
