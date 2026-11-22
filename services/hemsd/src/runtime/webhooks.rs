//! Events out: the box tells the systems a household names what it told the
//! household.
//!
//! The box speaks first to people (`push`); a household's automation and its
//! installer are the other listeners, and they want the fact with its numbers,
//! not a sentence. So the push detector's events leave by a second door as
//! well: **CloudEvents 1.0**, structured JSON, signed per **Standard Webhooks**
//! with a secret the box issues per subscription — the wire the fleet already
//! speaks, and one Home Assistant, Node-RED and every webhook library read.
//!
//! # What it promises
//!
//! - **At least once, with a stable id.** A delivery not answered `2xx` is
//!   tried again with growing gaps for 24 hours, then dropped and logged. The
//!   id is the CloudEvent's and the `webhook-id`, so a receiver de-duplicates.
//!   What is owed is kept in the subscription's own row, so a restart owes it
//!   still.
//! - **Nothing blocks the control loop.** Deliveries run on their own task,
//!   each bounded in time, and read at most [`MOST_BODY`] of an answer.
//! - **Only where the household pointed it.** A private address over `http` or
//!   `https`; a public one over `https` only and only when the household said
//!   so in words; never loopback, link-local, unspecified or multicast — checked
//!   when the subscription is made **and** when each delivery resolves, so a
//!   name that later resolves somewhere else is refused then. No redirect is
//!   followed.

use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use tokio::sync::Mutex;

use crate::runtime::push::{Event as Detected, Signals};
use crate::store::Store;

/// The most subscriptions a box keeps.
pub const AT_MOST: u64 = 8;

/// How long a delivery is tried before it is dropped.
pub const GIVE_UP_AFTER: Duration = Duration::hours(24);

/// How long the log remembers a delivery.
pub const LOG_KEEPS: Duration = Duration::days(7);

/// The most a subscription is owed at once; beyond it the oldest is dropped.
const MOST_OWED: usize = 200;

/// The most log entries a subscription keeps.
const MOST_LOGGED: usize = 500;

/// The most of a receiver's answer that is read.
pub const MOST_BODY: usize = 1024;

/// How long one attempt may take, connection to answer.
const ATTEMPT: std::time::Duration = std::time::Duration::from_secs(10);

/// The gaps between attempts. The last repeats until [`GIVE_UP_AFTER`].
const BACKOFF_S: [i64; 6] = [30, 120, 600, 1_800, 3_600, 10_800];

/// Every type this box sends to a subscriber, and nothing else.
pub const EMITTED: [&str; 7] = [
    ems_events::GRID_LPC_LIMIT_RECEIVED,
    ems_events::GRID_LPC_LIMIT_RELEASED,
    ems_events::GRID_LPC_FAILSAFE_ENTERED,
    ems_events::SITE_GOAL_AT_RISK,
    ems_events::DEVICE_DEVIATION_DETECTED,
    ems_events::SITE_DAY_CLOSED,
    ems_events::TEST_PING,
];

// ── The data each event carries ─────────────────────────────────────────────

/// `de.ems.grid.lpc.limit.received`: the network operator reduced the
/// connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LimitReceived {
    /// The ceiling on the controllable devices, W.
    pub limit_w: f64,
    /// What § 14a guarantees the household meanwhile, W.
    pub minimum_w: Option<f64>,
}

/// `de.ems.grid.lpc.limit.released`: the reduction ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LimitReleased {
    /// How long it lasted, seconds, where the box saw it begin.
    pub lasted_s: Option<i64>,
}

/// `de.ems.grid.lpc.failsafe.entered`: the control box fell silent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FailsafeEntered {
    /// The failsafe ceiling now in force, W.
    pub failsafe_w: Option<f64>,
}

/// `de.ems.site.goal.at-risk`: a goal turned tight or impossible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GoalAtRisk {
    /// `tight` or `impossible`.
    pub verdict: String,
    /// The devices whose goal it is.
    pub assets: Vec<String>,
}

/// `de.ems.device.deviation.detected`: a device has not done what it was
/// asked for ten minutes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeviationDetected {
    /// The devices not following.
    pub assets: Vec<String>,
}

/// `de.ems.test.ping`: sent on request, to check a receiver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TestPing {
    /// Always `true`; a receiver can tell a test from an event by its type.
    pub test: bool,
}

// ── Subscriptions ───────────────────────────────────────────────────────────

/// What a household asks for.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct Asked {
    /// A name the household recognises: *Home Assistant*, *the installer*.
    pub name: String,
    /// Where to deliver.
    pub url: String,
    /// The types wanted; a trailing `*` matches a family (`de.ems.grid.*`).
    pub types: Vec<String>,
    /// Narrow to these subjects — asset ids or the site; empty is all.
    #[serde(default)]
    pub subjects: Vec<String>,
    /// Required, and `true`, for an address on the public internet: the
    /// household has been told the events leave its network.
    #[serde(default)]
    pub public: bool,
}

/// One subscription as the box keeps it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Kept {
    id: String,
    name: String,
    url: String,
    types: Vec<String>,
    subjects: Vec<String>,
    /// Whether the household said the events may leave its network. Checked
    /// again on every attempt: a private name that later resolves to a public
    /// address is refused then.
    #[serde(default)]
    public: bool,
    /// The signing key, the bytes behind the `whsec_` the household was shown.
    key: Vec<u8>,
    created_at: i64,
    owed: VecDeque<Owed>,
    log: VecDeque<Delivery>,
}

/// One delivery still owed.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Owed {
    id: String,
    r#type: String,
    body: String,
    first: i64,
    next: i64,
    attempts: u32,
}

/// What the household sees of a subscription — never its secret.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct View {
    /// Its id, for `revoke` and `test`.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Where it delivers.
    pub url: String,
    /// The types it takes.
    pub types: Vec<String>,
    /// The subjects it narrows to; empty is all.
    pub subjects: Vec<String>,
    /// The last four characters of its secret, to tell two apart.
    pub secret_hint: String,
    /// When it was made, RFC 3339.
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "String")]
    pub created_at: OffsetDateTime,
    /// Deliveries still being tried.
    pub owed: usize,
}

/// A new subscription, with the secret shown this once.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct Created {
    /// The subscription.
    #[serde(flatten)]
    pub view: View,
    /// The Standard Webhooks secret, `whsec_…`. Shown once; the box keeps only
    /// what it signs with.
    pub secret: String,
}

/// One attempt, as the log keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Delivery {
    /// The subscription it was for.
    pub subscription: String,
    /// The event's id — the same on every attempt.
    pub event: String,
    /// The event's type.
    pub r#type: String,
    /// When it was tried, RFC 3339.
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "String")]
    pub at: OffsetDateTime,
    /// Which attempt, from 1.
    pub attempt: u32,
    /// The receiver's status, where it answered.
    pub status: Option<u16>,
    /// Why it failed, where it did.
    pub error: Option<String>,
    /// `delivered`, `retrying` or `dropped`.
    pub outcome: String,
}

/// What a test delivery found.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct TestDelivery {
    /// Whether the receiver answered `2xx`.
    pub delivered: bool,
    /// Its status, where it answered.
    pub status: Option<u16>,
    /// Why not, where not.
    pub error: Option<String>,
    /// How long it took.
    pub millis: u64,
}

/// Why a subscription was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    /// Not a URL the box can deliver to.
    #[error("{0}")]
    Destination(String),
    /// A type the box does not send.
    #[error("the box sends no event of type {0}")]
    UnknownType(String),
    /// There are [`AT_MOST`] already.
    #[error("this box already tells {AT_MOST} systems; revoke one first")]
    Full,
    /// The store failed.
    #[error("it could not be kept: {0}")]
    Store(String),
}

// ── Where a delivery may go ─────────────────────────────────────────────────

/// Whether `ip` may receive a delivery at all, and if so whether it is on the
/// household's own network.
fn reachable(ip: IpAddr) -> Result<bool, String> {
    let refuse = |why: &str| Err(format!("{ip} is {why}; the box does not deliver there"));
    if ip.is_loopback() {
        return refuse("this box itself");
    }
    if ip.is_unspecified() || ip.is_multicast() {
        return refuse("not one host");
    }
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_link_local() || v4.is_broadcast() {
                return refuse("link-local");
            }
            Ok(v4.is_private())
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            if first & 0xffc0 == 0xfe80 {
                return refuse("link-local");
            }
            if let Some(v4) = v6.to_ipv4_mapped() {
                return reachable(IpAddr::V4(v4));
            }
            // Unique local addresses, fc00::/7.
            Ok(first & 0xfe00 == 0xfc00)
        }
    }
}

/// Check a destination against the household's own words: a private address
/// over either scheme, a public one over `https` and only when `public`.
///
/// # Errors
/// [`Refused::Destination`] with the sentence the page shows.
pub fn destination(url: &str, addresses: &[IpAddr], public: bool) -> Result<reqwest::Url, Refused> {
    checked(url, addresses, public, false)
}

fn checked(
    url: &str,
    addresses: &[IpAddr],
    public: bool,
    loopback: bool,
) -> Result<reqwest::Url, Refused> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| Refused::Destination(format!("{url} is not a URL")))?;
    let https = match parsed.scheme() {
        "https" => true,
        "http" => false,
        other => {
            return Err(Refused::Destination(format!(
                "{other}:// is not a scheme the box delivers over; use https or http"
            )));
        }
    };
    if parsed.host().is_none() || addresses.is_empty() {
        return Err(Refused::Destination(format!(
            "{url} names no host the box can find"
        )));
    }
    let mut every_private = true;
    for ip in addresses {
        if loopback && ip.is_loopback() {
            continue;
        }
        every_private &= reachable(*ip).map_err(Refused::Destination)?;
    }
    if !every_private {
        if !https {
            return Err(Refused::Destination(
                "an address on the internet is delivered to over https only".into(),
            ));
        }
        if !public {
            return Err(Refused::Destination(
                "this address is on the internet: the events would leave your network — \
                 say so with `public: true`"
                    .into(),
            ));
        }
    }
    Ok(parsed)
}

async fn resolve(url: &reqwest::Url) -> Vec<SocketAddr> {
    let Some(host) = url.host_str() else {
        return Vec::new();
    };
    let port = url.port_or_known_default().unwrap_or(443);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    match tokio::net::lookup_host((host, port)).await {
        Ok(found) => found.collect(),
        Err(_) => Vec::new(),
    }
}

// ── The handle ──────────────────────────────────────────────────────────────

/// The webhook outlet: subscriptions, what they are owed, and delivery.
#[derive(Clone)]
pub struct Webhooks {
    store: Option<Arc<Mutex<Store>>>,
    /// Held across every read-modify-write of a subscription, so a delivery
    /// pass cannot write back a subscription that was revoked meanwhile, or
    /// drop an event queued meanwhile. Never held across a delivery.
    writing: Arc<Mutex<()>>,
    /// Deliver to loopback as well — for a receiver in a test on this machine.
    loopback: bool,
    memory: Arc<Mutex<Vec<Kept>>>,
    source: String,
    state: Arc<Mutex<Watch>>,
}

/// What the emitter remembers between observations.
#[derive(Default)]
struct Watch {
    reduced_since: Option<OffsetDateTime>,
    last_day: Option<String>,
}

impl Webhooks {
    /// For the site `site`, kept in `store` where there is one.
    #[must_use]
    pub fn new(site: &str, store: Option<Arc<Mutex<Store>>>) -> Self {
        Self {
            store,
            writing: Arc::new(Mutex::new(())),
            loopback: false,
            memory: Arc::new(Mutex::new(Vec::new())),
            source: format!("ems://sites/{site}"),
            state: Arc::new(Mutex::new(Watch::default())),
        }
    }

    /// Deliver to this machine's loopback too. For a test receiver; a box
    /// never delivers to itself.
    #[must_use]
    pub fn allowing_loopback_for_testing(mut self) -> Self {
        self.loopback = true;
        self
    }

    async fn all(&self) -> Result<Vec<Kept>, Refused> {
        match &self.store {
            Some(store) => store
                .lock()
                .await
                .webhooks::<Kept>()
                .map_err(|e| Refused::Store(e.to_string())),
            None => Ok(self.memory.lock().await.clone()),
        }
    }

    async fn put(&self, kept: &Kept) -> Result<bool, Refused> {
        if let Some(store) = &self.store {
            return store
                .lock()
                .await
                .put_webhook(&kept.id, kept, AT_MOST)
                .map_err(|e| Refused::Store(e.to_string()));
        }
        let mut memory = self.memory.lock().await;
        if let Some(old) = memory.iter_mut().find(|k| k.id == kept.id) {
            *old = kept.clone();
            return Ok(true);
        }
        if memory.len() as u64 >= AT_MOST {
            return Ok(false);
        }
        memory.push(kept.clone());
        Ok(true)
    }

    /// Every subscription, as the household sees it.
    ///
    /// # Errors
    /// [`Refused::Store`].
    pub async fn list(&self) -> Result<Vec<View>, Refused> {
        Ok(self.all().await?.iter().map(view).collect())
    }

    /// Make one, and hand back its secret once.
    ///
    /// # Errors
    /// [`Refused`], each with the sentence the page shows.
    pub async fn subscribe(&self, asked: Asked, now: OffsetDateTime) -> Result<Created, Refused> {
        for wanted in &asked.types {
            if !EMITTED.iter().any(|t| ems_events::matches(wanted, t)) {
                return Err(Refused::UnknownType(wanted.clone()));
            }
        }
        if asked.types.is_empty() {
            return Err(Refused::UnknownType("(none asked for)".into()));
        }
        let probe = reqwest::Url::parse(&asked.url)
            .map_err(|_| Refused::Destination(format!("{} is not a URL", asked.url)))?;
        let addresses: Vec<IpAddr> = resolve(&probe).await.iter().map(SocketAddr::ip).collect();
        checked(&asked.url, &addresses, asked.public, self.loopback)?;

        let mut key = vec![0_u8; 32];
        aws_lc_rs::rand::fill(&mut key).map_err(|_| Refused::Store("no randomness".into()))?;
        let secret = format!(
            "whsec_{}",
            base64::engine::general_purpose::STANDARD.encode(&key)
        );
        let kept = Kept {
            id: new_id(),
            name: asked.name.chars().take(64).collect(),
            url: asked.url,
            types: asked.types,
            subjects: asked.subjects,
            public: asked.public,
            key,
            created_at: now.unix_timestamp(),
            owed: VecDeque::new(),
            log: VecDeque::new(),
        };
        let kept_it = {
            let _writing = self.writing.lock().await;
            self.put(&kept).await?
        };
        if !kept_it {
            return Err(Refused::Full);
        }
        Ok(Created {
            view: view(&kept),
            secret,
        })
    }

    /// Stop delivering to one, and forget what it was owed.
    ///
    /// # Errors
    /// [`Refused::Store`]. `Ok(false)` where there was no such subscription.
    pub async fn revoke(&self, id: &str) -> Result<bool, Refused> {
        let _writing = self.writing.lock().await;
        if let Some(store) = &self.store {
            return store
                .lock()
                .await
                .forget_webhook(id)
                .map_err(|e| Refused::Store(e.to_string()));
        }
        let mut memory = self.memory.lock().await;
        let before = memory.len();
        memory.retain(|k| k.id != id);
        Ok(memory.len() < before)
    }

    /// Change the subscription `id` as it is now, under the write lock.
    /// `false` where it is gone — revoked while a delivery was out.
    async fn change(&self, id: &str, apply: impl FnOnce(&mut Kept)) -> Result<bool, Refused> {
        let _writing = self.writing.lock().await;
        let Some(mut kept) = self.all().await?.into_iter().find(|k| k.id == id) else {
            return Ok(false);
        };
        apply(&mut kept);
        self.put(&kept).await
    }

    /// The last seven days of attempts, newest first.
    ///
    /// # Errors
    /// [`Refused::Store`].
    pub async fn deliveries(&self, now: OffsetDateTime) -> Result<Vec<Delivery>, Refused> {
        let mut out: Vec<Delivery> = self
            .all()
            .await?
            .into_iter()
            .flat_map(|k| k.log.into_iter())
            .filter(|d| now - d.at <= LOG_KEEPS)
            .collect();
        out.sort_by_key(|d| std::cmp::Reverse(d.at));
        Ok(out)
    }

    /// Send a `de.ems.test.ping` now and say what the receiver answered.
    ///
    /// # Errors
    /// [`Refused::Store`]. `Ok(None)` where there is no such subscription.
    pub async fn test(
        &self,
        id: &str,
        now: OffsetDateTime,
    ) -> Result<Option<TestDelivery>, Refused> {
        let Some(kept) = self.all().await?.into_iter().find(|k| k.id == id) else {
            return Ok(None);
        };
        let event = ems_events::envelope::Event::new(
            ems_events::TEST_PING,
            self.source.clone(),
            new_id(),
            now,
            TestPing { test: true },
        );
        let body = event.to_bytes().unwrap_or_default();
        let started = std::time::Instant::now();
        let (status, error) = attempt(&kept, &event.id, &body, now, self.loopback).await;
        let millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let delivered = status.is_some_and(|s| (200..300).contains(&s));
        let entry = Delivery {
            subscription: kept.id.clone(),
            event: event.id.clone(),
            r#type: ems_events::TEST_PING.into(),
            at: now,
            attempt: 1,
            status,
            error: error.clone(),
            outcome: if delivered { "delivered" } else { "dropped" }.into(),
        };
        self.change(&kept.id, |k| log(k, entry, now)).await?;
        Ok(Some(TestDelivery {
            delivered,
            status,
            error,
            millis,
        }))
    }

    /// Turn what the push detector found into events, and owe them to every
    /// subscription that asked for their type and subject.
    ///
    /// One detector, two doors: a reduction is detected once and leaves both
    /// as a notification and as a CloudEvent.
    pub async fn emit(&self, detected: &[Detected], home: &serde_json::Value, now: OffsetDateTime) {
        let signals = Signals::of(home);
        let mut out: Vec<(&'static str, Option<String>, serde_json::Value)> = Vec::new();
        {
            let mut watch = self.state.lock().await;
            for event in detected {
                match event {
                    Detected::ReductionBegan => {
                        watch.reduced_since = Some(now);
                        let r = &home["reduction"];
                        out.push((
                            ems_events::GRID_LPC_LIMIT_RECEIVED,
                            None,
                            json(&LimitReceived {
                                limit_w: r["ceiling_kw"].as_f64().unwrap_or(0.0) * 1000.0,
                                minimum_w: r["minimum_kw"].as_f64().map(|kw| kw * 1000.0),
                            }),
                        ));
                    }
                    Detected::ReductionEnded => {
                        let lasted_s = watch
                            .reduced_since
                            .take()
                            .map(|t| (now - t).whole_seconds());
                        out.push((
                            ems_events::GRID_LPC_LIMIT_RELEASED,
                            None,
                            json(&LimitReleased { lasted_s }),
                        ));
                    }
                    Detected::Failsafe => out.push((
                        ems_events::GRID_LPC_FAILSAFE_ENTERED,
                        None,
                        json(&FailsafeEntered {
                            failsafe_w: home["reduction"]["ceiling_kw"]
                                .as_f64()
                                .map(|kw| kw * 1000.0),
                        }),
                    )),
                    Detected::GoalTight | Detected::GoalImpossible => {
                        let verdict = if matches!(event, Detected::GoalTight) {
                            "tight"
                        } else {
                            "impossible"
                        };
                        let assets: Vec<String> = signals.goals.keys().cloned().collect();
                        out.push((
                            ems_events::SITE_GOAL_AT_RISK,
                            assets.first().cloned(),
                            json(&GoalAtRisk {
                                verdict: verdict.into(),
                                assets,
                            }),
                        ));
                    }
                    Detected::DeviceNotFollowing => {
                        let assets: Vec<String> = signals.not_following.iter().cloned().collect();
                        out.push((
                            ems_events::DEVICE_DEVIATION_DETECTED,
                            assets.first().cloned(),
                            json(&DeviationDetected { assets }),
                        ));
                    }
                    Detected::Test => {}
                }
            }
        }
        for (r#type, subject, data) in out {
            self.owe(r#type, subject, data, now).await;
        }
    }

    /// Owe the day that just closed, once, to whoever asked for it.
    pub async fn day_closed(&self, day: &crate::month::ClosedDay, now: OffsetDateTime) {
        {
            let mut watch = self.state.lock().await;
            match &watch.last_day {
                // The first look is where the box is, not news: a restart does
                // not report yesterday a second time.
                None => {
                    watch.last_day = Some(day.day.clone());
                    return;
                }
                Some(seen) if *seen >= day.day => return,
                Some(_) => watch.last_day = Some(day.day.clone()),
            }
        }
        self.owe(
            ems_events::SITE_DAY_CLOSED,
            Some(day.day.clone()),
            json(day),
            now,
        )
        .await;
    }

    async fn owe(
        &self,
        r#type: &'static str,
        subject: Option<String>,
        data: serde_json::Value,
        now: OffsetDateTime,
    ) {
        let _writing = self.writing.lock().await;
        let Ok(all) = self.all().await else {
            return;
        };
        let mut event =
            ems_events::envelope::Event::new(r#type, self.source.clone(), new_id(), now, data);
        if let Some(subject) = &subject {
            event = event.about(subject.clone());
        }
        let Ok(body) = event.to_bytes() else {
            return;
        };
        let body = String::from_utf8_lossy(&body).into_owned();
        for mut kept in all {
            let wanted = kept.types.iter().any(|t| ems_events::matches(t, r#type));
            let about = kept.subjects.is_empty()
                || subject.as_ref().is_some_and(|s| kept.subjects.contains(s));
            if !wanted || !about {
                continue;
            }
            if kept.owed.len() >= MOST_OWED {
                kept.owed.pop_front();
            }
            kept.owed.push_back(Owed {
                id: event.id.clone(),
                r#type: r#type.into(),
                body: body.clone(),
                first: now.unix_timestamp(),
                next: now.unix_timestamp(),
                attempts: 0,
            });
            if let Err(error) = self.put(&kept).await {
                tracing::warn!(%error, "a webhook delivery could not be queued");
            }
        }
    }

    /// Deliver whatever is due. Called by [`run`].
    pub async fn deliver_due(&self, now: OffsetDateTime) {
        let Ok(all) = self.all().await else {
            return;
        };
        for kept in all {
            let due: Vec<Owed> = kept
                .owed
                .iter()
                .filter(|o| o.next <= now.unix_timestamp())
                .cloned()
                .collect();
            for owed in due {
                // The attempt runs with no lock held: a receiver that takes ten
                // seconds must not hold up a household revoking it.
                let (status, error) =
                    attempt(&kept, &owed.id, owed.body.as_bytes(), now, self.loopback).await;
                let delivered = status.is_some_and(|s| (200..300).contains(&s));
                let first = OffsetDateTime::from_unix_timestamp(owed.first).unwrap_or(now);
                let gap = BACKOFF_S[usize::try_from(owed.attempts)
                    .unwrap_or(usize::MAX)
                    .min(BACKOFF_S.len() - 1)];
                let give_up = !delivered && now + Duration::seconds(gap) - first > GIVE_UP_AFTER;
                let outcome = if delivered {
                    "delivered"
                } else if give_up {
                    tracing::warn!(subscription = %kept.name, event = %owed.id, "a webhook delivery was dropped after 24 hours");
                    "dropped"
                } else {
                    "retrying"
                };
                let entry = Delivery {
                    subscription: kept.id.clone(),
                    event: owed.id.clone(),
                    r#type: owed.r#type.clone(),
                    at: now,
                    attempt: owed.attempts + 1,
                    status,
                    error,
                    outcome: outcome.into(),
                };
                // Recorded on the subscription as it is now, not as it was read:
                // one revoked meanwhile stays revoked, and what was queued
                // meanwhile stays queued.
                let recorded = self
                    .change(&kept.id, |k| {
                        if let Some(slot) = k.owed.iter_mut().find(|o| o.id == owed.id) {
                            slot.attempts = owed.attempts + 1;
                            slot.next = now.unix_timestamp() + gap;
                        }
                        if delivered || give_up {
                            k.owed.retain(|o| o.id != owed.id);
                        }
                        log(k, entry, now);
                    })
                    .await;
                match recorded {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        tracing::warn!(%error, "a webhook delivery could not be recorded");
                    }
                }
            }
        }
    }
}

/// A random UUID (version 4), from the provider the box already links.
fn new_id() -> String {
    let mut b = [0_u8; 16];
    let _ = aws_lc_rs::rand::fill(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn json<T: Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

fn view(kept: &Kept) -> View {
    let secret = base64::engine::general_purpose::STANDARD.encode(&kept.key);
    View {
        id: kept.id.clone(),
        name: kept.name.clone(),
        url: kept.url.clone(),
        types: kept.types.clone(),
        subjects: kept.subjects.clone(),
        secret_hint: secret
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        created_at: OffsetDateTime::from_unix_timestamp(kept.created_at)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH),
        owed: kept.owed.len(),
    }
}

fn log(kept: &mut Kept, entry: Delivery, now: OffsetDateTime) {
    kept.log.retain(|d| now - d.at <= LOG_KEEPS);
    if kept.log.len() >= MOST_LOGGED {
        kept.log.pop_front();
    }
    kept.log.push_back(entry);
}

/// One POST: resolved, checked again, signed, bounded. The status where the
/// receiver answered, and why not where it did not.
async fn attempt(
    kept: &Kept,
    id: &str,
    body: &[u8],
    now: OffsetDateTime,
    loopback: bool,
) -> (Option<u16>, Option<String>) {
    let Ok(url) = reqwest::Url::parse(&kept.url) else {
        return (None, Some("the subscription's URL no longer parses".into()));
    };
    // Resolved and checked on every attempt, and the request pinned to what was
    // checked: a name that has since moved to this box or to link-local is
    // refused now, not merely when it was added.
    let addresses = resolve(&url).await;
    let ips: Vec<IpAddr> = addresses.iter().map(SocketAddr::ip).collect();
    if let Err(refused) = checked(&kept.url, &ips, kept.public, loopback) {
        return (None, Some(refused.to_string()));
    }
    let signed = ems_events::webhook::sign(&kept.key, id, now, body);
    let mut client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(ATTEMPT);
    if let (Some(host), Some(first)) = (url.host_str(), addresses.first()) {
        client = client.resolve(host, *first);
    }
    let Ok(client) = client.build() else {
        return (None, Some("no HTTP client".into()));
    };
    let mut request = client
        .post(url)
        .header("content-type", "application/cloudevents+json")
        .body(body.to_vec());
    for (name, value) in signed.headers() {
        request = request.header(name, value);
    }
    match request.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            // Read no more than a sentence of the answer, and discard it.
            let mut response = response;
            let mut read = 0;
            while let Ok(Some(chunk)) = response.chunk().await {
                read += chunk.len();
                if read >= MOST_BODY {
                    break;
                }
            }
            let error =
                (!(200..300).contains(&status)).then(|| format!("the receiver answered {status}"));
            (Some(status), error)
        }
        Err(error) => (
            None,
            Some(if error.is_timeout() {
                "the receiver did not answer in time".into()
            } else {
                "the receiver could not be reached".into()
            }),
        ),
    }
}

/// Deliver what is owed, every few seconds, until the box stops.
pub async fn run(hooks: Webhooks, shutdown: ems_service::Shutdown) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let stop = shutdown.wait();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            () = &mut stop => return,
            _ = tick.tick() => {}
        }
        let now = OffsetDateTime::now_utc();
        if let Some(store) = &hooks.store {
            let latest = store
                .lock()
                .await
                .closed_days()
                .ok()
                .and_then(|d| d.into_iter().max_by(|a, b| a.day.cmp(&b.day)));
            if let Some(day) = latest {
                hooks.day_closed(&day, now).await;
            }
        }
        hooks.deliver_due(now).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn the_household_network_is_reachable_and_the_box_itself_is_not() {
        assert_eq!(reachable(ip("192.168.1.20")), Ok(true));
        assert_eq!(reachable(ip("10.0.0.5")), Ok(true));
        assert_eq!(reachable(ip("fd00::5")), Ok(true));
        assert_eq!(reachable(ip("93.184.216.34")), Ok(false));
        for refused in [
            "127.0.0.1",
            "::1",
            "169.254.1.1",
            "fe80::1",
            "0.0.0.0",
            "224.0.0.1",
            "::ffff:127.0.0.1",
        ] {
            assert!(reachable(ip(refused)).is_err(), "{refused}");
        }
    }

    #[test]
    fn the_internet_takes_https_and_the_households_word() {
        let lan = [ip("192.168.1.20")];
        let net = [ip("93.184.216.34")];
        assert!(destination("http://ha.local:8123/api/webhook/x", &lan, false).is_ok());
        assert!(
            destination("http://example.org/hook", &net, true).is_err(),
            "plain http to the internet"
        );
        let unsaid = destination("https://example.org/hook", &net, false).unwrap_err();
        assert!(
            unsaid.to_string().contains("leave your network"),
            "{unsaid}"
        );
        assert!(destination("https://example.org/hook", &net, true).is_ok());
        assert!(destination("ftp://x/hook", &lan, false).is_err());
        assert!(destination("http://localhost/hook", &[ip("127.0.0.1")], true).is_err());
    }

    /// The automation in the docs names a type the box sends, and the fields it
    /// reads are the ones that type carries.
    #[test]
    fn the_home_assistant_automation_in_the_docs_matches_what_is_sent() {
        let page = include_str!("../../../../site/content/docs/api.md");
        let at = page.find("trigger.json.type == '").expect("the automation") + 22;
        let named = &page[at..at + page[at..].find('\'').expect("its end")];
        assert!(EMITTED.contains(&named), "{named}");
        let fields = schemars::schema_for!(LimitReceived);
        let properties = fields.get("properties").expect("properties");
        for read in ["limit_w", "minimum_w"] {
            assert!(
                page.contains(&format!("trigger.json.data.{read}")),
                "{read}"
            );
            assert!(properties.get(read).is_some(), "{read} is not in the event");
        }
    }

    #[test]
    fn every_emitted_type_is_in_the_catalogue() {
        for t in EMITTED {
            assert!(ems_events::is_known(t), "{t}");
        }
    }
}
