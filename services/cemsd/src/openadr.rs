//! One synthetic customer programme, a loopback VTN and its industrial VEN.
use anyhow::{Context, Result, bail, ensure};
use ems_sim::peak::{Moment, Trace, projected_mean};
use openadr::{
    core::{Clock, IntervalExpander, Timeline},
    model::{
        BlVenRequest, Event, EventPayloadDescriptor, EventRequest, Interval, IntervalPeriod,
        ObjectId, Program, ProgramRequest, ReadingType, Report, ReportDescriptor,
        ReportPayloadDescriptor, ReportRequest, ReportResource, StartTime, Timestamp, Value,
        ValuesMap, VenRequest,
    },
    vtn::{Vtn, auth::StaticTokenAuth, store::MemoryStorage},
};
use reqwest::{Client, Method, RequestBuilder, redirect::Policy};
use rust_decimal::Decimal;
use serde::{Serialize, de::DeserializeOwned};
use std::sync::{Arc, Mutex};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const SITE: &str = "shift-meter";
const PROGRAM: &str = "shift-mean-import";
const MEAN: &str = "EMS_QUARTER_MEAN_LIMIT";

fn timestamp(at: OffsetDateTime) -> Result<Timestamp> {
    Ok(Timestamp::from_second(at.unix_timestamp())?)
}

fn quarter_event(program_id: ObjectId, start: Timestamp, limit_kw: i32) -> Result<EventRequest> {
    let mut event = EventRequest::new(program_id)
        .with_targets(vec![SITE.parse()?])
        .with_interval_period(IntervalPeriod::new(StartTime::At(start), "PT15M".parse()?))
        .with_intervals(vec![Interval::new(
            0,
            vec![ValuesMap::new(
                MEAN.parse()?,
                vec![Value::Integer(limit_kw.into())],
            )],
        )]);
    event.payload_descriptors = Some(vec![
        EventPayloadDescriptor::new(MEAN.parse()?).with_units("KW".parse()?),
    ]);
    event.report_descriptors = Some(vec![ReportDescriptor {
        units: Some("KWH".parse()?),
        reading_type: Some(ReadingType::DirectRead),
        ..ReportDescriptor::new("USAGE".parse()?)
    }]);
    Ok(event)
}

// This fixture supports one exact programme profile. In particular, the standard
// IMPORT_CAPACITY_LIMIT is a site maximum and cannot stand in for an interval mean.
fn accepted_limit(
    event: &Event,
    program: &ObjectId,
    trace: &Trace,
    cursor: OffsetDateTime,
) -> Result<Option<i32>> {
    let start = timestamp(trace.start)?;
    let close = timestamp(trace.start + Duration::minutes(15))?;
    let timeline = Timeline::build(
        [(&event.id, &event.content)],
        &IntervalExpander::at(start),
        start,
        close,
    );
    if timeline.at(timestamp(cursor)?).is_none() {
        return Ok(None);
    }
    let event = &event.content;
    let limit = event
        .intervals
        .as_deref()
        .and_then(|v| v.first())
        .and_then(|i| i.payloads.first())
        .and_then(|v| v.values.first())
        .and_then(Value::as_integer)
        .and_then(|v| i32::try_from(v).ok())
        .filter(|v| *v >= 0)
        .context("expected a nonnegative whole-kW quarter mean")?;
    ensure!(
        event == &quarter_event(program.clone(), start, limit)?,
        "unsupported programme, target, interval, units or report terms"
    );
    Ok(Some(limit))
}

fn metered_report(
    trace: &Trace,
    at: OffsetDateTime,
    event_id: ObjectId,
) -> Result<Option<ReportRequest>> {
    if at < trace.start + Duration::minutes(15) {
        return Ok(None);
    }
    let Some(wh) = trace.first_reconciled_wh() else {
        return Ok(None);
    };
    let usage = ValuesMap::new(
        "USAGE".parse()?,
        vec![Value::Number(Decimal::from(wh) / Decimal::from(1000))],
    );
    let mut report = ReportRequest::new(
        event_id,
        SITE.parse()?,
        vec![ReportResource {
            resource_name: SITE.parse()?,
            interval_period: Some(IntervalPeriod::new(
                StartTime::At(timestamp(trace.start)?),
                "PT15M".parse()?,
            )),
            intervals: vec![Interval::new(0, vec![usage])],
        }],
    );
    let mut descriptor = ReportPayloadDescriptor::new("USAGE".parse()?);
    descriptor.units = Some("KWH".parse()?);
    descriptor.reading_type = Some(ReadingType::DirectRead);
    report.payload_descriptors = Some(vec![descriptor]);
    Ok(Some(report))
}

async fn exchange<T: DeserializeOwned>(request: RequestBuilder) -> Result<T> {
    Ok(request.send().await?.error_for_status()?.json().await?)
}

#[derive(Serialize)]
struct Receipt {
    simulated: bool,
    #[serde(with = "time::serde::rfc3339")]
    interval_start: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    interval_end: OffsetDateTime,
    cancelled_before_dispatch: bool,
    requested_mean_kw: Option<i32>,
    projected_early_mean_kw: Option<f64>,
    projected_delayed_mean_kw: Option<f64>,
    reported_meter_kwh: Option<f64>,
    original_meter_mean_kw: Option<f64>,
}

#[derive(Debug)]
struct ReplayClock(Mutex<Timestamp>);

impl Clock for ReplayClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().expect("replay clock")
    }
}

impl ReplayClock {
    fn advance(&self, at: OffsetDateTime) -> Result<()> {
        *self.0.lock().expect("replay clock") = timestamp(at)?;
        Ok(())
    }
}

struct Pilot {
    clock: Arc<ReplayClock>,
    client: Client,
    base: String,
    business_token: Uuid,
    ven_token: Uuid,
}

impl Pilot {
    fn business(&self, method: reqwest::Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(self.business_token)
    }
    fn ven(&self, method: reqwest::Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(self.ven_token)
    }

    async fn events(&self, program: &ObjectId) -> Result<Vec<Event>> {
        exchange(
            self.ven(reqwest::Method::GET, "/events")
                .query(&[("programID", program.as_str()), ("targets", SITE)]),
        )
        .await
    }

    async fn decision(
        &self,
        trace: &Trace,
        moment: Moment,
        program: &ObjectId,
        event_id: &ObjectId,
    ) -> Result<(Option<i32>, Option<f64>)> {
        let at = moment.at(trace.start);
        self.clock.advance(at)?;
        let current = self.events(program).await?;
        let limit = match current.as_slice() {
            [] => None,
            [event] if &event.id == event_id => accepted_limit(event, program, trace, at)?,
            _ => bail!("unexpected events at the dispatch cursor"),
        };
        Ok((
            limit,
            limit.and_then(|kw| projected_mean(trace, moment, kw)),
        ))
    }
    async fn rehearse(&self, trace: &Trace, cancel: bool) -> Result<Receipt> {
        self.business(Method::POST, "/vens")
            .json(&VenRequest::Bl(BlVenRequest {
                client_id: SITE.parse()?,
                ven_name: SITE.parse()?,
                targets: vec![SITE.parse()?],
                attributes: None,
            }))
            .send()
            .await?
            .error_for_status()?;
        let program: Program = exchange(
            self.business(Method::POST, "/programs")
                .json(&ProgramRequest::new(PROGRAM.parse()?).with_targets(vec![SITE.parse()?])),
        )
        .await?;
        let start = timestamp(trace.start)?;
        let published: Event =
            exchange(self.business(Method::POST, "/events").json(&quarter_event(
                program.id.clone(),
                start,
                1100,
            )?))
            .await?;
        let scheduled = self.events(&program.id).await?;
        ensure!(
            matches!(scheduled.as_slice(), [event] if event.id == published.id),
            "VEN did not receive its event"
        );
        self.clock.advance(trace.start + Duration::minutes(9))?;
        if cancel {
            self.business(Method::DELETE, &format!("/events/{}", published.id))
                .send()
                .await?
                .error_for_status()?;
        }
        let (limit, early) = self
            .decision(trace, Moment::Early, &program.id, &published.id)
            .await?;
        let (delayed_limit, delayed) = self
            .decision(trace, Moment::Delayed, &program.id, &published.id)
            .await?;
        ensure!(
            limit == delayed_limit && limit.is_some() != cancel,
            "unexpected target after polling"
        );
        let close = trace.start + Duration::minutes(15);
        self.clock.advance(close)?;
        let mut energy = None;
        if limit.is_some()
            && let Some(request) = metered_report(trace, close, published.id.clone())?
        {
            let filed: Report = exchange(self.ven(Method::POST, "/reports").json(&request)).await?;
            let stored: Report =
                exchange(self.business(Method::GET, &format!("/reports/{}", filed.id))).await?;
            // The value's wire representation may change from decimal to integer.
            let expected: ReportRequest = serde_json::from_value(serde_json::to_value(&request)?)?;
            ensure!(
                stored.created_date_time == timestamp(close)?
                    && stored.content == expected
                    && stored
                        .client_id
                        .as_ref()
                        .is_some_and(|id| id.as_str() == SITE),
                "report readback differs from the VEN meter submission"
            );
            energy = stored.content.resources[0]
                .intervals
                .first()
                .and_then(|i| i.payloads.first())
                .and_then(|v| v.values.first())
                .and_then(Value::as_decimal)
                .and_then(|v| rust_decimal::prelude::ToPrimitive::to_f64(&v));
            ensure!(energy.is_some(), "missing reported energy");
        }
        let reports: Vec<Report> = exchange(
            self.business(Method::GET, "/reports")
                .query(&[("eventID", published.id.as_str())]),
        )
        .await?;
        ensure!(
            reports.len() == usize::from(energy.is_some()),
            "unexpected number of reports"
        );
        Ok(Receipt {
            simulated: true,
            interval_start: trace.start,
            interval_end: trace
                .register
                .iter()
                .find(|sample| sample.at == close)
                .map_or(close.to_offset(time::UtcOffset::UTC), |sample| sample.at),
            cancelled_before_dispatch: cancel,
            requested_mean_kw: limit,
            projected_early_mean_kw: early,
            projected_delayed_mean_kw: delayed,
            reported_meter_kwh: energy,
            original_meter_mean_kw: energy.map(|kwh| kwh * 4.0),
        })
    }
}

async fn simulate(trace: &Trace, cancel: bool) -> Result<Receipt> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let base = format!("http://{}/openadr3/3.1.0", listener.local_addr()?);
    let pilot = Pilot {
        clock: Arc::new(ReplayClock(Mutex::new(timestamp(trace.start)?))),
        client: Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .build()?,
        base,
        business_token: Uuid::new_v4(),
        ven_token: Uuid::new_v4(),
    };
    let auth = StaticTokenAuth::new(format!("{}/auth/token", pilot.base))
        .with_business_logic(
            pilot.business_token.to_string(),
            "programme-operator".parse()?,
        )
        .with_ven(pilot.ven_token.to_string(), SITE.parse()?);
    let router = Vtn::builder()
        .storage(MemoryStorage::shared())
        .authenticator(Arc::new(auth))
        .clock(pilot.clock.clone())
        .build()
        .router();
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let result = pilot.rehearse(trace, cancel).await;
    server.abort();
    result
}

fn site_time(at: OffsetDateTime) -> String {
    format!(
        "{:02}:{:02} UTC{:+}",
        at.hour(),
        at.minute(),
        at.offset().whole_hours()
    )
}

pub async fn run(cancel: bool, json: bool) -> Result<()> {
    let receipt = simulate(&Trace::bundled()?, cancel).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&receipt)?);
    } else {
        println!(
            "OpenADR 3.1 · simulated industrial quarter · {} · {} to {}",
            receipt.interval_start.date(),
            site_time(receipt.interval_start),
            site_time(receipt.interval_end)
        );
        if let Some(kw) = receipt.requested_mean_kw {
            println!("Requested quarter-hour mean: {kw} kW");
            for (moment, mean) in [
                (Moment::Early, receipt.projected_early_mean_kw),
                (Moment::Delayed, receipt.projected_delayed_mean_kw),
            ] {
                let at = moment.at(receipt.interval_start);
                let label = site_time(at);
                match mean {
                    Some(kw) => println!("Projected mean, battery starts at {label}: {kw} kW"),
                    None => println!("Projection unavailable at {label}"),
                }
            }
            if let (Some(kwh), Some(kw)) =
                (receipt.reported_meter_kwh, receipt.original_meter_mean_kw)
            {
                println!("Meter report, original battery-idle trace: {kwh} kWh ({kw} kW mean)");
            } else {
                println!("Meter report pending reconciliation");
            }
        } else {
            println!("Event withdrawn before dispatch. No active target or usage report.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn projected_dispatch_and_reported_usage_remain_distinct() {
        let receipt = simulate(&Trace::bundled().unwrap(), false).await.unwrap();
        assert_eq!(receipt.requested_mean_kw, Some(1100));
        assert_eq!(receipt.projected_early_mean_kw, Some(1100.0));
        assert_eq!(receipt.projected_delayed_mean_kw, Some(1116.0));
        assert_eq!(receipt.reported_meter_kwh, Some(285.0));
        assert_eq!(receipt.original_meter_mean_kw, Some(1140.0));
    }

    #[tokio::test]
    async fn withdrawing_the_event_clears_the_proposal_and_report() {
        let receipt = simulate(&Trace::bundled().unwrap(), true).await.unwrap();
        assert!(receipt.cancelled_before_dispatch);
        assert!(receipt.requested_mean_kw.is_none());
        assert!(
            receipt.projected_early_mean_kw.is_none()
                && receipt.projected_delayed_mean_kw.is_none()
        );
        assert!(receipt.reported_meter_kwh.is_none() && receipt.original_meter_mean_kw.is_none());
    }

    #[test]
    fn reports_require_a_closed_and_reconciled_meter_interval() {
        let mut trace = Trace::bundled().unwrap();
        let close = trace.start + Duration::minutes(15);
        let id = "shift-event".parse::<ObjectId>().unwrap();
        assert!(
            metered_report(&trace, close - Duration::seconds(1), id.clone())
                .unwrap()
                .is_none()
        );
        trace.closed.clear();
        assert!(metered_report(&trace, close, id).unwrap().is_none());
    }

    #[test]
    fn the_profile_checks_target_units_quarter_and_programme() {
        let trace = Trace::bundled().unwrap();
        let program: ObjectId = "programme".parse().unwrap();
        let start = timestamp(trace.start).unwrap();
        let request = quarter_event(program.clone(), start, 1200).unwrap();
        let event = Event {
            id: "event".parse().unwrap(),
            object_type: openadr::model::ObjectType::Event,
            created_date_time: start,
            modification_date_time: start,
            content: request,
        };
        let cursor = trace.start + Duration::minutes(10);
        assert_eq!(
            accepted_limit(&event, &program, &trace, cursor).unwrap(),
            Some(1200)
        );
        for moment in [Moment::Early, Moment::Delayed] {
            assert_eq!(projected_mean(&trace, moment, 1200), Some(1140.0));
        }
        for at in [
            trace.start - Duration::seconds(1),
            trace.start + Duration::minutes(15),
        ] {
            assert!(
                accepted_limit(&event, &program, &trace, at)
                    .unwrap()
                    .is_none()
            );
        }
        assert!(accepted_limit(&event, &"other".parse().unwrap(), &trace, cursor).is_err());
        let mut other = event.clone();
        other.content.targets = vec!["other".parse().unwrap()];
        assert!(accepted_limit(&other, &program, &trace, cursor).is_err());
        other = event.clone();
        other.content.payload_descriptors.as_mut().unwrap()[0].units = Some("KWH".parse().unwrap());
        assert!(accepted_limit(&other, &program, &trace, cursor).is_err());
        other = event;
        other.content.intervals.as_mut().unwrap()[0].payloads[0].value_type =
            "IMPORT_CAPACITY_LIMIT".parse().unwrap();
        assert!(accepted_limit(&other, &program, &trace, cursor).is_err());
    }
}
