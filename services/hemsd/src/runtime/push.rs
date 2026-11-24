//! The box speaks first: Web Push for the four things worth an interruption.
//!
//! A household that has to open a page to learn its car will not be ready has
//! been told too late. A box that notifies about everything is muted by the
//! second week. So exactly four kinds of event, each one something a person
//! would do something about, each opt-in per browser:
//!
//! | Kind | Notifies |
//! |---|---|
//! | `reduction` | the network operator's § 14a reduction begins, and ends |
//! | `goal` | a goal turns *tight* or *impossible* while there is time to change it |
//! | `device` | a device has not followed the box for ten minutes |
//! | `failsafe` | the control box fell silent and the stored failsafe value applies |
//!
//! # What leaves the house
//!
//! One HTTPS request to the browser vendor's push service, carrying a message
//! only that browser can read (`[RFC 8291]`, [`super::webpush`]) and a signature
//! only this box can make (`[RFC 8292]`). The message is a sentence and the view
//! to open — never a power, a price or a device's name — so what the push
//! service learns is that this box said something to this browser, and when.
//!
//! The message is the *Declarative Web Push* document, which Safari shows on
//! its own and every other browser hands to the page's small service worker.
//!
//! # Where a subscription may point
//!
//! The endpoint is a URL the subscribing browser supplies, and the box will
//! `POST` to it — so an endpoint that could be anything would turn the box into
//! a way of reaching the household's own network from a browser tab. It may
//! therefore only be `https` on the default port at one of the push services
//! browsers actually use ([`PUSH_SERVICES`]), and the client follows no redirect.
//!
//! # What is deliberately not here
//!
//! No queue. A reduction that ended an hour ago is not news, so a message the
//! box cannot send is dropped, and one a push service cannot deliver within
//! [`TTL_S`] is dropped there. Quiet hours are the one deferral, and what they
//! defer is sent only if it is still true when they end.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use tokio::sync::Mutex;

use super::webpush::{self, Vapid};

/// How many browsers one box will notify.
///
/// A household's phones and a tablet, with room. A bound, because every row is
/// an outbound request per event and a store nobody else trims.
pub const MAX_SUBSCRIPTIONS: u64 = 8;

/// How long a device may ignore the box before the household is told.
///
/// Ten minutes: longer than a heat pump's start-up sequence or a charge point
/// renegotiating phases, shorter than the quarter hour a plan slot costs.
pub const NOT_FOLLOWING_AFTER: Duration = Duration::minutes(10);

/// How long a goal must be out of trouble before trouble is news again.
///
/// A verdict is a threshold on a ratio, and a ratio sitting on its threshold
/// crosses it every few minutes. One notification per episode means the episode
/// has to be allowed to end.
pub const GOAL_REARM: Duration = Duration::minutes(30);

/// How long a push service may hold a message it cannot deliver, seconds.
pub const TTL_S: u32 = 1800;

/// The push services a subscription may point at, by host suffix.
///
/// Apple (Safari, every browser on iOS), Google's FCM under both names Chrome
/// issues (Chrome, Brave, Samsung Internet, Vivaldi, Opera), Mozilla's autopush
/// (Firefox) and Microsoft's WNS (Edge) — the production hosts of the
/// community-kept `pushpad/known-push-services` list. A browser on another
/// service is refused by name rather than trusted by default.
pub const PUSH_SERVICES: [&str; 5] = [
    "push.apple.com",
    "fcm.googleapis.com",
    "jmt17.google.com",
    "push.services.mozilla.com",
    "notify.windows.com",
];

/// Who a push service may contact about this sender — `[RFC 8292 2.1]`.
///
/// The project, not the household: the claim is readable by the push service,
/// and a household's address has no business in it.
const CONTACT: &str = env!("CARGO_PKG_REPOSITORY");

/// The four things a browser can ask to be told about.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "PushEvent")]
pub enum Kind {
    /// A § 14a reduction begins or ends.
    Reduction,
    /// A goal turns tight or impossible.
    Goal,
    /// A device stops following.
    Device,
    /// The box enters failsafe.
    Failsafe,
}

impl Kind {
    /// All four.
    pub const ALL: [Self; 4] = [Self::Reduction, Self::Goal, Self::Device, Self::Failsafe];
}

/// One thing that happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Event {
    /// The network operator reduced the connection.
    ReductionBegan,
    /// And lifted it.
    ReductionEnded,
    /// A goal is reachable only if nothing else needs the connection.
    GoalTight,
    /// A goal is not reachable by its time.
    GoalImpossible,
    /// A device has ignored the box for [`NOT_FOLLOWING_AFTER`].
    DeviceNotFollowing,
    /// The control box is silent and the failsafe value applies.
    Failsafe,
    /// The household pressed *send a test*.
    Test,
}

/// An episode keeps its identity across delivery channels and quiet hours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occurrence {
    /// What changed.
    pub event: Event,
    /// The CloudEvent ID, also carried in the encrypted notification.
    pub id: uuid::Uuid,
}

impl From<Event> for Occurrence {
    fn from(event: Event) -> Self {
        Self {
            event,
            id: uuid::Uuid::new_v4(),
        }
    }
}

impl Event {
    /// Every event, for the tests that hold a table to it.
    pub const ALL: [Self; 7] = [
        Self::ReductionBegan,
        Self::ReductionEnded,
        Self::GoalTight,
        Self::GoalImpossible,
        Self::DeviceNotFollowing,
        Self::Failsafe,
        Self::Test,
    ];

    /// Which opt-in covers it — `None` for the test, which is asked for by hand.
    #[must_use]
    pub const fn kind(self) -> Option<Kind> {
        Some(match self {
            Self::ReductionBegan | Self::ReductionEnded => Kind::Reduction,
            Self::GoalTight | Self::GoalImpossible => Kind::Goal,
            Self::DeviceNotFollowing => Kind::Device,
            Self::Failsafe => Kind::Failsafe,
            Self::Test => return None,
        })
    }

    /// The view a tap on it opens: where the household can act on it.
    ///
    /// A goal at risk opens Plan, where the goal is; failsafe opens Setup,
    /// whose trust lines say what the control box last did; a reduction and a
    /// device that stopped following open Now, which says what the house is
    /// doing about them.
    #[must_use]
    pub const fn view(self) -> &'static str {
        match self {
            Self::GoalTight | Self::GoalImpossible => "plan",
            Self::Failsafe => "setup",
            Self::ReductionBegan | Self::ReductionEnded | Self::DeviceNotFollowing | Self::Test => {
                "now"
            }
        }
    }

    /// The `Topic` a push service collapses undelivered messages by, and the
    /// tag a browser replaces a shown notification by — so *ended* replaces
    /// *began* on a phone that was off for both.
    #[must_use]
    pub const fn topic(self) -> &'static str {
        match self {
            Self::ReductionBegan | Self::ReductionEnded | Self::Failsafe => "ems-grid",
            Self::GoalTight | Self::GoalImpossible => "hems-goal",
            Self::DeviceNotFollowing => "ems-device",
            Self::Test => "hems-test",
        }
    }

    /// Whether what it says still holds — asked when quiet hours end.
    #[must_use]
    pub fn still_true(self, signals: &Signals) -> bool {
        match self {
            Self::ReductionBegan => signals.grid == Grid::Reduced,
            Self::Failsafe => signals.grid == Grid::Failsafe,
            Self::GoalTight => signals.goals.values().any(|g| g.risk == Some(Risk::Tight)),
            Self::GoalImpossible => signals
                .goals
                .values()
                .any(|g| g.risk == Some(Risk::Impossible)),
            Self::DeviceNotFollowing => !signals.not_following.is_empty(),
            // An end is never deferred: by morning it is old news.
            Self::ReductionEnded | Self::Test => false,
        }
    }

    /// The notification, as a title and one sentence.
    ///
    /// Box-side and fixed: a notification is read on a lock screen with the
    /// page closed, so there is no page to own the wording, and a sentence with
    /// no number in it is one a push service's logs cannot leak.
    #[must_use]
    pub const fn sentence(self, lang: Lang) -> (&'static str, &'static str) {
        match (self, lang) {
            (Self::ReductionBegan, Lang::De) => (
                "Der Netzbetreiber dimmt den Anschluss",
                "Steuerbare Geräte laufen gedrosselt, bis er die Reduzierung aufhebt. Das Haus bleibt versorgt.",
            ),
            (Self::ReductionBegan, Lang::En) => (
                "The network operator is reducing the connection",
                "Controllable devices run throttled until it lifts the reduction. The house stays supplied.",
            ),
            (Self::ReductionEnded, Lang::De) => (
                "Die Reduzierung ist beendet",
                "Der Netzbetreiber hat den Anschluss wieder freigegeben. Alles läuft nach Plan weiter.",
            ),
            (Self::ReductionEnded, Lang::En) => (
                "The reduction has ended",
                "The network operator has released the connection. Everything is back on plan.",
            ),
            (Self::GoalTight, Lang::De) => (
                "Ein Ziel wird knapp",
                "Es ist nur noch erreichbar, wenn nichts dazwischenkommt. Noch lässt es sich anpassen.",
            ),
            (Self::GoalTight, Lang::En) => (
                "A goal is getting tight",
                "It can only be met if nothing else gets in the way. There is still time to change it.",
            ),
            (Self::GoalImpossible, Lang::De) => (
                "Ein Ziel ist nicht mehr erreichbar",
                "Bis zur gewählten Zeit reicht es nicht. Die Box zeigt, was stattdessen erreichbar ist.",
            ),
            (Self::GoalImpossible, Lang::En) => (
                "A goal can no longer be met",
                "There is not enough time left. The box shows what can be reached instead.",
            ),
            (Self::DeviceNotFollowing, Lang::De) => (
                "Ein Gerät folgt der Box nicht",
                "Seit über zehn Minuten setzt ein Gerät nicht um, was die Box vorgibt. Die Box zeigt, welches.",
            ),
            (Self::DeviceNotFollowing, Lang::En) => (
                "A device is not following the box",
                "For over ten minutes a device has not done what the box asked. The box shows which one.",
            ),
            (Self::Failsafe, Lang::De) => (
                "Die Steuerbox meldet sich nicht",
                "Die Box hält den hinterlegten Sicherheitswert ein, bis die Steuerbox des Netzbetreibers wieder erreichbar ist.",
            ),
            (Self::Failsafe, Lang::En) => (
                "The control box has gone silent",
                "The box keeps to the stored failsafe value until the network operator's control box is heard from again.",
            ),
            (Self::Test, Lang::De) => (
                "So meldet sich die Box",
                "Nur, wenn es etwas zu tun gibt — sonst bleibt sie still.",
            ),
            (Self::Test, Lang::En) => (
                "This is how the box speaks",
                "Only when there is something to do — otherwise it stays quiet.",
            ),
        }
    }
}

/// The language a browser reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "PushLang")]
pub enum Lang {
    /// German.
    De,
    /// English.
    En,
}

impl Lang {
    /// The BCP 47 tag.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::De => "de",
            Self::En => "en",
        }
    }
}

/// The Declarative Web Push document for one event.
///
/// `web_push: 8030` is what makes a browser that understands the format show
/// it without running anything; `navigate` is required by it and is where a tap
/// lands — the view the event is about. Others hand the same document to the
/// page's service worker.
#[must_use]
pub fn message(occurrence: Occurrence, lang: Lang, origin: &str) -> serde_json::Value {
    let event = occurrence.event;
    let (title, body) = event.sentence(lang);
    serde_json::json!({
        "web_push": 8030,
        "notification": {
            "title": title,
            "body": body,
            "lang": lang.tag(),
            "dir": "ltr",
            "tag": event.topic(),
            "data": { "event_id": occurrence.id },
            "navigate": format!("{origin}/#{}", event.view()),
        }
    })
}

// ── What the box watches ─────────────────────────────────────────────────────

/// What the network operator's side of the connection is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Grid {
    /// Nothing asked.
    #[default]
    Free,
    /// A reduction is in force.
    Reduced,
    /// The control box is silent and the failsafe value applies.
    Failsafe,
}

/// How much trouble a goal is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    /// Possible only if nothing else needs the connection.
    Tight,
    /// Not possible by then.
    Impossible,
}

/// One goal, as far as a notification cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Goal {
    /// What the household asked for, as text: a change here is the household
    /// changing its mind, which it does not need to be told about.
    pub asked: String,
    /// The trouble it is in, if any.
    pub risk: Option<Risk>,
}

/// Everything the four kinds are decided from, at one instant.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Signals {
    /// The operator's side.
    pub grid: Grid,
    /// Every goal in force, by where the page shows it.
    pub goals: BTreeMap<String, Goal>,
    /// The devices not doing what the box asked, or not answering it.
    pub not_following: BTreeSet<String>,
}

impl Signals {
    /// Read them from the household page's own document.
    ///
    /// **From the document rather than from the state behind it**, on purpose: a
    /// notification is a sentence the page would show, sent early, and reading
    /// the same JSON is what stops the two from ever disagreeing about whether
    /// a reduction is in force or a goal is tight.
    #[must_use]
    pub fn of(home: &serde_json::Value) -> Self {
        let grid = match home.get("reduction") {
            Some(r) if r.is_object() => {
                if r.get("failsafe").and_then(serde_json::Value::as_bool) == Some(true) {
                    Grid::Failsafe
                } else {
                    Grid::Reduced
                }
            }
            _ => Grid::Free,
        };
        let not_following = home
            .get("devices")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|d| {
                matches!(
                    d.get("notice").and_then(serde_json::Value::as_str),
                    Some("disobedient" | "silent")
                )
            })
            .filter_map(|d| d.get("id").and_then(serde_json::Value::as_str))
            .map(str::to_owned)
            .collect();
        // A goal is whatever part of the document carries a verdict — the
        // car's, the tank's, and the next kind without a change here. A member
        // with one verdict is one goal; a member with several is a list or a
        // map of them.
        let mut goals = BTreeMap::new();
        for (key, value) in home.as_object().into_iter().flatten() {
            match verdicts_in(value) {
                0 => {}
                1 => {
                    goals.insert(key.clone(), goal_of(value));
                }
                _ => {
                    let several: Vec<(String, &serde_json::Value)> = match value {
                        serde_json::Value::Array(items) => items
                            .iter()
                            .enumerate()
                            .map(|(n, v)| (n.to_string(), v))
                            .collect(),
                        serde_json::Value::Object(map) => {
                            map.iter().map(|(name, v)| (name.clone(), v)).collect()
                        }
                        _ => Vec::new(),
                    };
                    for (name, item) in several {
                        if verdicts_in(item) > 0 {
                            goals.insert(format!("{key}/{name}"), goal_of(item));
                        }
                    }
                }
            }
        }
        Self {
            grid,
            goals,
            not_following,
        }
    }
}

/// How many verdicts there are beneath `value`.
fn verdicts_in(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(map) => {
            usize::from(map.get("verdict").is_some_and(serde_json::Value::is_string))
                + map.values().map(verdicts_in).sum::<usize>()
        }
        serde_json::Value::Array(items) => items.iter().map(verdicts_in).sum(),
        _ => 0,
    }
}

/// The first verdict beneath `value`.
fn verdict_in(value: &serde_json::Value) -> Option<&str> {
    match value {
        serde_json::Value::Object(map) => map
            .get("verdict")
            .and_then(serde_json::Value::as_str)
            .or_else(|| map.values().find_map(verdict_in)),
        serde_json::Value::Array(items) => items.iter().find_map(verdict_in),
        _ => None,
    }
}

/// One goal: what was asked (its targets and its time) and the trouble it is in.
fn goal_of(value: &serde_json::Value) -> Goal {
    fn asked(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, v) in map {
                    let names_the_ask = key.starts_with("target")
                        || key.starts_with("by_")
                        || matches!(key.as_str(), "asset" | "id" | "kind");
                    if names_the_ask && !v.is_object() && !v.is_array() {
                        out.push(format!("{key}={v}"));
                    } else {
                        asked(v, out);
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| asked(v, out)),
            _ => {}
        }
    }
    let mut parts = Vec::new();
    asked(value, &mut parts);
    Goal {
        asked: parts.join(" "),
        risk: match verdict_in(value) {
            Some("tight") => Some(Risk::Tight),
            Some("impossible") => Some(Risk::Impossible),
            _ => None,
        },
    }
}

/// Turns a stream of [`Signals`] into the few [`Event`]s worth sending.
///
/// Pure: it is given the time. **One event per episode** is the whole job — the
/// control loop ticks every second and a household is told once.
#[derive(Debug, Default)]
pub struct Detector {
    grid: Option<Grid>,
    goals: BTreeMap<String, GoalEpisode>,
    failing_since: BTreeMap<String, (OffsetDateTime, bool)>,
    device_notification_sent: bool,
}

#[derive(Debug)]
struct GoalEpisode {
    asked: String,
    /// The worst the household has been told in this episode.
    told: Option<Risk>,
    /// Since when the goal has been out of trouble, while an episode is open.
    clear_since: Option<OffsetDateTime>,
}

impl Detector {
    /// What changed between the last observation and this one.
    ///
    /// The first observation changes nothing: a box that restarts in the middle
    /// of a reduction has nothing new to say about it.
    pub fn observe(&mut self, signals: &Signals, now: OffsetDateTime) -> Vec<Event> {
        let changes = self.changes(signals, now);
        self.notifications(&changes)
            .into_iter()
            .map(|o| o.event)
            .collect()
    }

    fn notifications(&mut self, changes: &[(Occurrence, Option<String>)]) -> Vec<Occurrence> {
        if self.failing_since.is_empty() {
            self.device_notification_sent = false;
        }
        let mut events = Vec::new();
        for (occurrence, _) in changes {
            if occurrence.event == Event::DeviceNotFollowing
                && std::mem::replace(&mut self.device_notification_sent, true)
            {
                continue;
            }
            if !events
                .iter()
                .any(|o: &Occurrence| o.event == occurrence.event)
            {
                events.push(*occurrence);
            }
        }
        events
    }

    /// Changes with the goal key or device ID that triggered the report.
    pub fn changes(
        &mut self,
        signals: &Signals,
        now: OffsetDateTime,
    ) -> Vec<(Occurrence, Option<String>)> {
        let mut events = Vec::new();

        match (self.grid.replace(signals.grid), signals.grid) {
            (Some(Grid::Free), Grid::Reduced) => events.push((Event::ReductionBegan, None)),
            (Some(Grid::Free | Grid::Reduced), Grid::Failsafe) => {
                events.push((Event::Failsafe, None));
            }
            (Some(Grid::Reduced), Grid::Free) => events.push((Event::ReductionEnded, None)),
            // Out of failsafe is the control box coming back — into whatever it
            // then asks for, which the page shows. Not a fifth kind of event.
            _ => {}
        }

        self.goals.retain(|id, _| signals.goals.contains_key(id));
        for (id, goal) in &signals.goals {
            let known = self
                .goals
                .get_mut(id)
                .filter(|episode| episode.asked == goal.asked);
            let Some(episode) = known else {
                // New, or changed by the household: it has just been shown the
                // verdict on the page it set the goal from.
                self.goals.insert(
                    id.clone(),
                    GoalEpisode {
                        asked: goal.asked.clone(),
                        told: goal.risk,
                        clear_since: None,
                    },
                );
                continue;
            };
            match goal.risk {
                None => {
                    let since = *episode.clear_since.get_or_insert(now);
                    if now - since >= GOAL_REARM {
                        episode.told = None;
                    }
                }
                Some(risk) => {
                    episode.clear_since = None;
                    if episode.told < Some(risk) {
                        episode.told = Some(risk);
                        events.push((
                            match risk {
                                Risk::Tight => Event::GoalTight,
                                Risk::Impossible => Event::GoalImpossible,
                            },
                            Some(id.clone()),
                        ));
                    }
                }
            }
        }

        self.failing_since
            .retain(|id, _| signals.not_following.contains(id));
        for id in &signals.not_following {
            let (since, told) = self.failing_since.entry(id.clone()).or_insert((now, false));
            if !*told && now - *since >= NOT_FOLLOWING_AFTER {
                *told = true;
                events.push((Event::DeviceNotFollowing, Some(id.clone())));
            }
        }
        events
            .into_iter()
            .map(|(event, subject)| (Occurrence::from(event), subject))
            .collect()
    }
}

// ── Who is told ──────────────────────────────────────────────────────────────

/// The hours a browser does not want to hear from the box, local wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(rename = "QuietHours")]
pub struct Quiet {
    /// From this minute after midnight…
    pub from_minute: u16,
    /// …until this one. Smaller than `from_minute` when it spans midnight.
    pub to_minute: u16,
}

impl Quiet {
    /// Whether `minute` of the local day is inside them.
    #[must_use]
    pub const fn contains(self, minute: u16) -> bool {
        if self.from_minute <= self.to_minute {
            self.from_minute <= minute && minute < self.to_minute
        } else {
            minute >= self.from_minute || minute < self.to_minute
        }
    }
}

/// One browser the box may notify, as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    /// Where the browser's push service takes messages for it.
    pub endpoint: String,
    /// The browser's public key, base64url.
    pub p256dh: String,
    /// Its authentication secret, base64url.
    pub auth: String,
    /// The language it reads.
    pub lang: Lang,
    /// What it asked to be told about.
    pub events: BTreeSet<Kind>,
    /// When it does not want to be.
    pub quiet: Option<Quiet>,
    /// The address the household reaches the box at from this browser, so a
    /// tap opens the page there.
    pub origin: String,
    /// When it subscribed, RFC 3339.
    pub created: String,
}

/// What `POST /v1/push` takes: a browser's `PushSubscription.toJSON()` and the
/// household's choices.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[schemars(rename = "PushSubscribe")]
pub struct Asked {
    /// The push service's address for this browser.
    pub endpoint: String,
    /// The browser's keys.
    pub keys: AskedKeys,
    /// `de` or `en`. German where absent.
    #[serde(default)]
    pub lang: Option<Lang>,
    /// Which kinds. All four where absent.
    #[serde(default)]
    pub events: Option<BTreeSet<Kind>>,
    /// Quiet hours, where wanted.
    #[serde(default)]
    pub quiet: Option<Quiet>,
}

/// A subscription's keys, as a browser spells them.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[schemars(rename = "PushKeys")]
pub struct AskedKeys {
    /// The public key, base64url.
    pub p256dh: String,
    /// The authentication secret, base64url.
    pub auth: String,
}

/// A subscription, as the page is shown it: everything but the keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[schemars(rename = "PushSubscription")]
pub struct View {
    /// The endpoint — how the page finds its own browser in the list.
    pub endpoint: String,
    /// The language.
    pub lang: Lang,
    /// The kinds asked for.
    pub events: BTreeSet<Kind>,
    /// Quiet hours.
    pub quiet: Option<Quiet>,
    /// When it subscribed.
    pub created: String,
}

impl From<&Subscription> for View {
    fn from(s: &Subscription) -> Self {
        Self {
            endpoint: s.endpoint.clone(),
            lang: s.lang,
            events: s.events.clone(),
            quiet: s.quiet,
            created: s.created.clone(),
        }
    }
}

/// What `GET /v1/push` answers.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[schemars(rename = "PushOverview")]
pub struct Overview {
    /// The box's public key: the `applicationServerKey` a browser subscribes
    /// with, base64url.
    pub key: String,
    /// Every browser subscribed.
    pub subscriptions: Vec<View>,
    /// How many there may be.
    pub at_most: u64,
}

/// Why a subscription was not taken.
#[derive(Debug, thiserror::Error)]
pub enum Refused {
    /// The endpoint is not somewhere the box will send.
    #[error("{0}")]
    Endpoint(String),
    /// The keys are not a subscription's.
    #[error("{0}")]
    Keys(#[from] webpush::Error),
    /// The quiet hours are not hours.
    #[error("quiet hours are two different minutes of the day, each below 1440")]
    Quiet,
    /// The box already notifies [`MAX_SUBSCRIPTIONS`] browsers.
    #[error("this box already notifies {MAX_SUBSCRIPTIONS} browsers; remove one first")]
    Full,
    /// The store failed.
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
}

/// Which endpoints the box will send to.
#[derive(Debug, Clone, Default)]
pub enum Allow {
    /// [`PUSH_SERVICES`], over `https` on the default port.
    #[default]
    PushServices,
    /// One more origin, exactly — a test's own push service on loopback.
    #[cfg(test)]
    Also(String),
}

impl Allow {
    /// The push service's origin for `endpoint` — the VAPID audience — where
    /// the box will send there.
    ///
    /// # Errors
    /// A sentence for the household where it will not.
    pub fn audience(&self, endpoint: &str) -> Result<String, String> {
        if endpoint.len() > 2048 {
            return Err("the push endpoint is longer than any push service issues".into());
        }
        let url = reqwest::Url::parse(endpoint)
            .map_err(|_| "the push endpoint is not a URL".to_owned())?;
        let origin = url.origin().ascii_serialization();
        #[cfg(test)]
        if matches!(self, Self::Also(allowed) if *allowed == origin) {
            return Ok(origin);
        }
        let host = url.host_str().unwrap_or_default();
        let known = PUSH_SERVICES
            .iter()
            .any(|s| host == *s || host.ends_with(&format!(".{s}")));
        if url.scheme() != "https"
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || !known
        {
            return Err(format!(
                "`{host}` is not a push service this box sends to (Apple, Google, Mozilla, Microsoft, over https)"
            ));
        }
        Ok(origin)
    }
}

/// What became of one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The push service took it.
    Accepted,
    /// The subscription no longer exists (`404`, `410`); it has been forgotten.
    Gone,
    /// The push service refused it, with this status.
    Refused(u16),
    /// The push service could not be reached; nothing is retried.
    Unreachable,
}

/// The box's Web Push sender.
#[derive(Clone)]
pub struct Push {
    store: Arc<Mutex<crate::store::Store>>,
    vapid: Arc<Vapid>,
    http: reqwest::Client,
    allow: Allow,
}

impl Push {
    /// The sender over this box's store, with the signing key it kept — or a
    /// new one, kept from now on.
    ///
    /// # Errors
    /// Where the key cannot be read, made or kept, or no HTTP client can be built.
    pub async fn open(store: Arc<Mutex<crate::store::Store>>) -> anyhow::Result<Self> {
        Self::open_with(store, Allow::default()).await
    }

    async fn open_with(
        store: Arc<Mutex<crate::store::Store>>,
        allow: Allow,
    ) -> anyhow::Result<Self> {
        let document = {
            let store = store.lock().await;
            if let Some(kept) = store.push_vapid()? {
                kept
            } else {
                let made = Vapid::generate()?;
                store.put_push_vapid(&made)?;
                made
            }
        };
        let http = reqwest::Client::builder()
            // A push service answers `201` or an error. A redirect is not an
            // answer it gives, and following one is how a checked address
            // becomes an unchecked one.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(10))
            .user_agent(concat!("hemsd/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            store,
            vapid: Arc::new(Vapid::from_pkcs8(&document)?),
            http,
            allow,
        })
    }

    /// The key and the browsers subscribed.
    ///
    /// # Errors
    /// Where the store cannot be read.
    pub async fn overview(&self) -> Result<Overview, crate::store::StoreError> {
        Ok(Overview {
            key: self.vapid.public_key(),
            subscriptions: self.subscriptions().await?.iter().map(View::from).collect(),
            at_most: MAX_SUBSCRIPTIONS,
        })
    }

    async fn subscriptions(&self) -> Result<Vec<Subscription>, crate::store::StoreError> {
        self.store.lock().await.push_subscriptions()
    }

    /// Subscribe a browser, or change what a subscribed one asked for.
    ///
    /// `origin` is where that browser reaches the box.
    ///
    /// # Errors
    /// [`Refused`], with a sentence the page can show.
    pub async fn subscribe(
        &self,
        asked: Asked,
        origin: String,
        now: OffsetDateTime,
    ) -> Result<View, Refused> {
        self.allow
            .audience(&asked.endpoint)
            .map_err(Refused::Endpoint)?;
        let point = webpush::unb64(&asked.keys.p256dh).ok_or(webpush::Error::Key)?;
        let auth = webpush::unb64(&asked.keys.auth).ok_or(webpush::Error::Auth)?;
        // Encrypting to it is the check: the provider refuses a point that is
        // not on the curve, here rather than at the first reduction.
        webpush::encrypt(b"", &point, &auth)?;
        if asked.quiet.is_some_and(|q| {
            q.from_minute >= 1440 || q.to_minute >= 1440 || q.from_minute == q.to_minute
        }) {
            return Err(Refused::Quiet);
        }
        let subscription = Subscription {
            endpoint: asked.endpoint,
            p256dh: webpush::b64(&point),
            auth: webpush::b64(&auth),
            lang: asked.lang.unwrap_or(Lang::De),
            events: asked
                .events
                .unwrap_or_else(|| Kind::ALL.into_iter().collect()),
            quiet: asked.quiet,
            origin,
            created: now
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default(),
        };
        let kept = self.store.lock().await.put_push_subscription(
            &subscription.endpoint,
            &subscription,
            MAX_SUBSCRIPTIONS,
        )?;
        if kept {
            Ok(View::from(&subscription))
        } else {
            Err(Refused::Full)
        }
    }

    /// Forget a browser. Returns whether it was subscribed.
    ///
    /// # Errors
    /// Where the store cannot be written.
    pub async fn forget(&self, endpoint: &str) -> Result<bool, crate::store::StoreError> {
        self.store.lock().await.forget_push_subscription(endpoint)
    }

    /// Send the test notification to one subscribed browser — `None` where
    /// `endpoint` is not one.
    ///
    /// # Errors
    /// Where the store cannot be read.
    pub async fn test(
        &self,
        endpoint: &str,
        now: OffsetDateTime,
    ) -> Result<Option<Outcome>, crate::store::StoreError> {
        let all = self.subscriptions().await?;
        let Some(subscription) = all.iter().find(|s| s.endpoint == endpoint) else {
            return Ok(None);
        };
        Ok(Some(self.send(subscription, Event::Test.into(), now).await))
    }

    /// Send one event to one browser.
    ///
    /// Never an error: a message that cannot be sent is dropped and said so in
    /// the log, by the push service's host and never by the endpoint — the
    /// endpoint's path is the capability to reach that browser.
    pub async fn send(
        &self,
        subscription: &Subscription,
        occurrence: Occurrence,
        now: OffsetDateTime,
    ) -> Outcome {
        let event = occurrence.event;
        let host = reqwest::Url::parse(&subscription.endpoint)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        let prepared = self
            .allow
            .audience(&subscription.endpoint)
            .and_then(|audience| {
                let body = message(occurrence, subscription.lang, &subscription.origin).to_string();
                let sealed = webpush::encrypt(
                    body.as_bytes(),
                    &webpush::unb64(&subscription.p256dh).unwrap_or_default(),
                    &webpush::unb64(&subscription.auth).unwrap_or_default(),
                )
                .map_err(|e| e.to_string())?;
                // Twelve hours: `[RFC 8292 2]` allows a day, and half of one
                // tolerates a box whose clock is wrong by hours either way.
                let authorization = self
                    .vapid
                    .authorization(
                        &audience,
                        (now + Duration::hours(12)).unix_timestamp(),
                        CONTACT,
                    )
                    .map_err(|e| e.to_string())?;
                Ok((sealed, authorization))
            });
        let (sealed, authorization) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::warn!(%host, %error, ?event, "a notification could not be prepared");
                return Outcome::Refused(0);
            }
        };
        let sent = self
            .http
            .post(&subscription.endpoint)
            .header("TTL", TTL_S)
            .header("Urgency", "high")
            .header("Topic", event.topic())
            .header(reqwest::header::CONTENT_ENCODING, "aes128gcm")
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .header(reqwest::header::AUTHORIZATION, authorization)
            .body(sealed)
            .send()
            .await;
        match sent.map(|r| r.status().as_u16()) {
            Ok(200..=299) => {
                tracing::info!(%host, ?event, "a notification was sent");
                Outcome::Accepted
            }
            Ok(404 | 410) => {
                // `[RFC 8030 7.3]`: the subscription is gone and will not come
                // back, so neither does the box.
                if let Err(error) = self.forget(&subscription.endpoint).await {
                    tracing::warn!(%host, %error, "an expired subscription could not be forgotten");
                }
                tracing::info!(%host, "a browser's subscription has ended; forgotten");
                Outcome::Gone
            }
            Ok(status) => {
                tracing::warn!(%host, status, ?event, "a push service refused a notification");
                Outcome::Refused(status)
            }
            Err(error) => {
                tracing::warn!(%host, error = %error.without_url(), ?event,
                    "a push service could not be reached; the notification is dropped");
                Outcome::Unreachable
            }
        }
    }
}

/// What quiet hours are holding back, per browser.
#[derive(Debug, Default)]
pub struct Deferred {
    owed: BTreeMap<String, BTreeMap<Event, Occurrence>>,
}

impl Deferred {
    /// Which browser is sent which event now.
    ///
    /// A browser in its quiet hours is sent nothing; what it would have been
    /// sent is remembered, and sent when they end **if it is still true**. An
    /// end is never remembered — it cancels the beginning it ends.
    pub fn due<'a>(
        &mut self,
        subscriptions: &'a [Subscription],
        events: &[Occurrence],
        signals: &Signals,
        local_minute: u16,
    ) -> Vec<(&'a Subscription, Occurrence)> {
        self.owed
            .retain(|endpoint, _| subscriptions.iter().any(|s| s.endpoint == *endpoint));
        let mut out = Vec::new();
        for subscription in subscriptions {
            let wanted = |e: &Occurrence| {
                e.event
                    .kind()
                    .is_some_and(|k| subscription.events.contains(&k))
            };
            if subscription.quiet.is_some_and(|q| q.contains(local_minute)) {
                let owed = self.owed.entry(subscription.endpoint.clone()).or_default();
                for event in events.iter().copied().filter(wanted) {
                    match event.event {
                        Event::ReductionEnded => {
                            owed.remove(&Event::ReductionBegan);
                        }
                        other => {
                            owed.insert(other, event);
                        }
                    }
                }
                continue;
            }
            if let Some(owed) = self.owed.remove(&subscription.endpoint) {
                out.extend(
                    owed.into_values()
                        // The worse verdict speaks for both.
                        .filter(|e| {
                            e.event.still_true(signals)
                                && !events.iter().any(|o| o.event == e.event)
                        })
                        .map(|e| (subscription, e)),
                );
            }
            out.extend(
                events
                    .iter()
                    .copied()
                    .filter(wanted)
                    .map(|e| (subscription, e)),
            );
        }
        out
    }
}

/// The minute of the local day at `now`, on the Berlin wall clock every other
/// time of day in this box is on.
#[must_use]
pub fn local_minute(now: OffsetDateTime) -> u16 {
    let local = ems_core::calendar::to_berlin(now);
    u16::from(local.hour()) * 60 + u16::from(local.minute())
}

/// How often the watcher looks.
const EVERY: std::time::Duration = std::time::Duration::from_secs(15);

/// Watch the box and tell whoever asked what is worth telling: the subscribed
/// browsers a sentence, the subscribed systems the event with its numbers.
///
/// One detector, two doors. `document` yields the household page's document —
/// the same one `GET /v1/home` answers — or `None` while there is none to read.
pub async fn run<F, Fut>(
    push: Option<Push>,
    hooks: Option<crate::runtime::webhooks::Webhooks>,
    document: F,
    shutdown: ems_service::Shutdown,
) where
    F: Fn() -> Fut + Send,
    Fut: std::future::Future<Output = Option<serde_json::Value>> + Send,
{
    let mut detector = Detector::default();
    let mut deferred = Deferred::default();
    let mut tick = tokio::time::interval(EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let stop = shutdown.wait();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            () = &mut stop => return,
            _ = tick.tick() => {}
        }
        let Some(home) = document().await else {
            continue;
        };
        let now = OffsetDateTime::now_utc();
        let signals = Signals::of(&home);
        let changes = detector.changes(&signals, now);
        if let Some(hooks) = &hooks {
            hooks.emit(&changes, &home, now).await;
        }
        let Some(push) = &push else {
            continue;
        };
        let events = detector.notifications(&changes);
        let subscriptions = match push.subscriptions().await {
            Ok(subscriptions) => subscriptions,
            Err(error) => {
                tracing::warn!(%error, "the push subscriptions could not be read");
                continue;
            }
        };
        for (subscription, event) in
            deferred.due(&subscriptions, &events, &signals, local_minute(now))
        {
            push.send(subscription, event, now).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::webpush::receiver::Receiver;
    use time::macros::datetime;

    const T0: OffsetDateTime = datetime!(2026-11-19 16:00 UTC);
    /// What a detector says when there is nothing to say.
    const NOTHING: [Event; 0] = [];

    fn at(minutes: i64) -> OffsetDateTime {
        T0 + Duration::minutes(minutes)
    }

    fn home(reduction: Option<bool>, verdict: Option<&str>, notices: &[(&str, &str)]) -> Signals {
        Signals::of(&serde_json::json!({
            "reduction": reduction.map(|failsafe| serde_json::json!({
                "ceiling_kw": 4.2, "failsafe": failsafe, "shares": {}
            })),
            "devices": notices.iter().map(|(id, notice)| serde_json::json!({
                "id": id, "kind": "car", "notice": notice
            })).collect::<Vec<_>>(),
            "goal": verdict.map(|v| serde_json::json!([
                {"target_soc": 0.8, "by_minute": 420},
                {"verdict": v, "needed_kwh": 12.0, "tightness": 0.9}
            ])),
        }))
    }

    #[test]
    fn every_event_has_a_sentence_in_both_languages_and_no_number_in_it() {
        for event in Event::ALL {
            let (de_title, de_body) = event.sentence(Lang::De);
            let (en_title, en_body) = event.sentence(Lang::En);
            for text in [de_title, de_body, en_title, en_body] {
                assert!(!text.is_empty(), "{event:?}");
                // FR-005: a sentence, never a measurement.
                assert!(
                    !text.chars().any(|c| c.is_ascii_digit()),
                    "{event:?} carries a figure: {text}"
                );
            }
            assert_ne!(de_body, en_body, "{event:?} is translated");
            assert!(event.topic().len() <= 32, "[RFC 8030 5.4]");
        }
        // Exactly four kinds, and every event but the test belongs to one.
        let kinds: BTreeSet<Kind> = Event::ALL.iter().filter_map(|e| e.kind()).collect();
        assert_eq!(kinds, Kind::ALL.into_iter().collect());
    }

    #[test]
    fn the_message_is_the_declarative_document_and_nothing_more() {
        let m = message(
            Event::ReductionBegan.into(),
            Lang::En,
            "https://hems.local:8443",
        );
        assert_eq!(m["web_push"], 8030);
        let n = m["notification"].as_object().unwrap();
        let keys: Vec<&str> = n.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["body", "data", "dir", "lang", "navigate", "tag", "title"]
        );
        assert_eq!(n["navigate"], "https://hems.local:8443/#now");
        assert_eq!(n["lang"], "en");
        assert_eq!(m.as_object().unwrap().len(), 2, "no measurement beside it");
    }

    /// A tap opens the view the household can act from, and only views the
    /// page has.
    #[test]
    fn a_notification_opens_what_it_is_about() {
        let opens =
            |e: Event| message(e.into(), Lang::En, "https://b")["notification"]["navigate"].clone();
        assert_eq!(opens(Event::GoalImpossible), "https://b/#plan");
        assert_eq!(opens(Event::GoalTight), "https://b/#plan");
        assert_eq!(opens(Event::Failsafe), "https://b/#setup");
        assert_eq!(opens(Event::ReductionBegan), "https://b/#now");
        assert_eq!(opens(Event::DeviceNotFollowing), "https://b/#now");
        let page = include_str!("../../ui/app.js");
        let views = &page[page.find("const VIEWS").expect("the page's views")..];
        let views = &views[..views.find(';').expect("their end")];
        for event in Event::ALL {
            assert!(
                views.contains(&format!("\"{}\"", event.view())),
                "{event:?}"
            );
        }
    }

    #[test]
    fn signals_are_read_from_the_page_s_own_document() {
        assert_eq!(home(None, None, &[]), Signals::default());
        assert_eq!(home(Some(false), None, &[]).grid, Grid::Reduced);
        assert_eq!(home(Some(true), None, &[]).grid, Grid::Failsafe);
        let s = home(
            None,
            Some("tight"),
            &[("ev", "disobedient"), ("hp", "silent"), ("pv", "assumed")],
        );
        assert_eq!(
            s.not_following,
            ["ev".to_owned(), "hp".to_owned()].into_iter().collect()
        );
        assert_eq!(
            s.goals["goal"],
            Goal {
                asked: "by_minute=420 target_soc=0.8".into(),
                risk: Some(Risk::Tight)
            }
        );
        assert_eq!(home(None, Some("feasible"), &[]).goals["goal"].risk, None);

        // The tank's goal beside the car's, as the page's document has them.
        let both = Signals::of(&serde_json::json!({
            "goal": [{"target_soc": 0.8, "by_minute": 420}, {"verdict": "feasible"}],
            "tank_goal": [
                {"target_c": 50.0, "by_minute": 1080},
                {"verdict": "impossible", "needed_kwh": 4.0, "reachable_c": 44.0}
            ],
            "receipt": {"saved_eur": 1.07},
        }));
        assert_eq!(both.goals.len(), 2);
        assert_eq!(
            both.goals["tank_goal"],
            Goal {
                asked: "by_minute=1080 target_c=50.0".into(),
                risk: Some(Risk::Impossible)
            }
        );

        // Several goals, however the document comes to carry them.
        let several = Signals::of(&serde_json::json!({ "goals": [
            {"asset": "ev", "target_soc": 0.8, "by_minute": 420, "answer": {"verdict": "impossible"}},
            {"asset": "tank", "target_c": 50.0, "by_minute": 1080, "answer": {"verdict": "met"}},
        ]}));
        assert_eq!(several.goals.len(), 2);
        assert_eq!(several.goals["goals/0"].risk, Some(Risk::Impossible));
        assert!(several.goals["goals/1"].asked.contains("target_c=50.0"));
        let named = Signals::of(&serde_json::json!({ "goals": {
            "ev": {"goal": {"target_soc": 0.8}, "answer": {"verdict": "tight"}},
            "tank": {"goal": {"target_c": 50.0}, "answer": {"verdict": "feasible"}},
        }}));
        assert_eq!(named.goals["goals/ev"].risk, Some(Risk::Tight));
        assert_eq!(named.goals["goals/tank"].risk, None);
    }

    #[test]
    fn the_names_read_here_are_the_ones_the_page_s_document_is_built_with() {
        // `Signals::of` reads JSON, so a renamed member would not fail to
        // compile — it would fall silent. This is what fails instead.
        let home = include_str!("../home.rs");
        for spelled in [
            "pub reduction: Option<Reduction>",
            "pub failsafe: bool",
            "pub devices: Vec<Device>",
            "pub notice: Option<&'static str>",
            "Some(\"silent\")",
            "Some(\"disobedient\")",
            "pub verdict: Verdict",
        ] {
            assert!(home.contains(spelled), "home.rs no longer has `{spelled}`");
        }
        let word = |v| serde_json::to_value(v).unwrap();
        assert_eq!(word(crate::home::Verdict::Tight), "tight");
        assert_eq!(word(crate::home::Verdict::Impossible), "impossible");
    }

    #[test]
    fn a_reduction_is_two_events_however_long_it_lasts() {
        let mut d = Detector::default();
        let mut seen = Vec::new();
        seen.extend(d.observe(&home(None, None, &[]), at(0)));
        for minute in 1..=90 {
            seen.extend(d.observe(&home(Some(false), None, &[]), at(minute)));
        }
        seen.extend(d.observe(&home(None, None, &[]), at(91)));
        seen.extend(d.observe(&home(None, None, &[]), at(92)));
        assert_eq!(seen, [Event::ReductionBegan, Event::ReductionEnded]);
    }

    #[test]
    fn a_box_that_restarts_into_a_reduction_says_nothing_new() {
        let mut d = Detector::default();
        assert_eq!(
            d.observe(&home(Some(false), Some("impossible"), &[]), at(0)),
            NOTHING
        );
        assert_eq!(
            d.observe(&home(None, Some("impossible"), &[]), at(1)),
            [Event::ReductionEnded]
        );
    }

    #[test]
    fn failsafe_is_its_own_event_and_not_a_reduction() {
        let mut d = Detector::default();
        d.observe(&home(None, None, &[]), at(0));
        assert_eq!(
            d.observe(&home(Some(true), None, &[]), at(1)),
            [Event::Failsafe]
        );
        // The control box comes back and asks for a reduction: the page shows
        // it; the household was told about this episode already.
        assert_eq!(d.observe(&home(Some(false), None, &[]), at(2)), NOTHING);
        assert_eq!(
            d.observe(&home(None, None, &[]), at(3)),
            [Event::ReductionEnded]
        );
        // A reduction that loses its control box is a failsafe.
        d.observe(&home(Some(false), None, &[]), at(4));
        assert_eq!(
            d.observe(&home(Some(true), None, &[]), at(5)),
            [Event::Failsafe]
        );
    }

    #[test]
    fn a_goal_is_told_once_per_episode_and_only_when_it_gets_worse() {
        let mut d = Detector::default();
        d.observe(&home(None, Some("feasible"), &[]), at(0));
        assert_eq!(
            d.observe(&home(None, Some("tight"), &[]), at(1)),
            [Event::GoalTight]
        );
        // Flapping on the threshold is one episode.
        for minute in 2..20 {
            let verdict = if minute % 2 == 0 { "feasible" } else { "tight" };
            assert_eq!(
                d.observe(&home(None, Some(verdict), &[]), at(minute)),
                NOTHING
            );
        }
        assert_eq!(
            d.observe(&home(None, Some("impossible"), &[]), at(20)),
            [Event::GoalImpossible]
        );
        // Better again is not news.
        assert_eq!(d.observe(&home(None, Some("tight"), &[]), at(21)), NOTHING);
        // Half an hour out of trouble ends the episode; then it is news again.
        assert_eq!(
            d.observe(&home(None, Some("feasible"), &[]), at(22)),
            NOTHING
        );
        assert_eq!(
            d.observe(&home(None, Some("feasible"), &[]), at(52)),
            NOTHING
        );
        assert_eq!(
            d.observe(&home(None, Some("tight"), &[]), at(53)),
            [Event::GoalTight]
        );
    }

    #[test]
    fn a_goal_the_household_just_set_or_changed_is_not_news_to_it() {
        let mut d = Detector::default();
        d.observe(&home(None, None, &[]), at(0));
        // Set, and impossible at once: the page said so when it was set.
        assert_eq!(
            d.observe(&home(None, Some("impossible"), &[]), at(1)),
            NOTHING
        );
        // Withdrawn and set again, feasible, then it turns.
        d.observe(&home(None, None, &[]), at(2));
        d.observe(&home(None, Some("feasible"), &[]), at(3));
        assert_eq!(
            d.observe(&home(None, Some("impossible"), &[]), at(4)),
            [Event::GoalImpossible]
        );
    }

    #[test]
    fn a_device_is_reported_after_ten_minutes_and_once() {
        let mut d = Detector::default();
        d.observe(&home(None, None, &[]), at(0));
        let failing = home(None, None, &[("ev", "disobedient")]);
        for minute in 1..10 {
            assert_eq!(d.observe(&failing, at(minute)), NOTHING, "{minute}");
        }
        // A second one joining does not restart the clock or tell twice.
        let both = home(None, None, &[("ev", "disobedient"), ("hp", "silent")]);
        assert_eq!(d.observe(&both, at(11)), [Event::DeviceNotFollowing]);
        assert_eq!(d.observe(&both, at(30)), NOTHING);
        // A hiccup shorter than ten minutes is never told.
        d.observe(&home(None, None, &[]), at(31));
        d.observe(&failing, at(32));
        assert_eq!(d.observe(&home(None, None, &[]), at(40)), NOTHING);
        assert_eq!(d.observe(&failing, at(41)), NOTHING);
        assert_eq!(d.observe(&failing, at(51)), [Event::DeviceNotFollowing]);
    }

    fn subscription(endpoint: &str, events: &[Kind], quiet: Option<Quiet>) -> Subscription {
        let r = Receiver::fresh();
        Subscription {
            endpoint: endpoint.into(),
            p256dh: webpush::b64(&r.public),
            auth: webpush::b64(&r.auth),
            lang: Lang::De,
            events: events.iter().copied().collect(),
            quiet,
            origin: "https://hems.local".into(),
            created: String::new(),
        }
    }

    #[test]
    fn each_browser_gets_what_it_asked_for() {
        let subs = [
            subscription("a", &Kind::ALL, None),
            subscription("b", &[Kind::Goal], None),
            subscription("c", &[], None),
        ];
        let mut deferred = Deferred::default();
        let due = deferred.due(
            &subs,
            &[Event::ReductionBegan.into(), Event::GoalTight.into()],
            &Signals::default(),
            600,
        );
        let got: Vec<(&str, Event)> = due
            .iter()
            .map(|(s, e)| (s.endpoint.as_str(), e.event))
            .collect();
        assert_eq!(
            got,
            [
                ("a", Event::ReductionBegan),
                ("a", Event::GoalTight),
                ("b", Event::GoalTight)
            ]
        );
    }

    #[test]
    fn quiet_hours_hold_a_reduction_until_morning_and_only_if_it_still_stands() {
        const NIGHT: Quiet = Quiet {
            from_minute: 22 * 60,
            to_minute: 7 * 60,
        };
        assert!(NIGHT.contains(23 * 60) && NIGHT.contains(0) && NIGHT.contains(6 * 60 + 59));
        assert!(!NIGHT.contains(7 * 60) && !NIGHT.contains(21 * 60 + 59));
        let day = Quiet {
            from_minute: 600,
            to_minute: 720,
        };
        assert!(day.contains(600) && !day.contains(720) && !day.contains(0));

        let subs = [subscription("a", &Kind::ALL, Some(NIGHT))];
        let reduced = home(Some(false), None, &[]);
        let free = home(None, None, &[]);
        let began = Occurrence::from(Event::ReductionBegan);

        // 23:00 it begins; still in force at 07:00.
        let mut deferred = Deferred::default();
        assert_eq!(deferred.due(&subs, &[began], &reduced, 23 * 60).len(), 0);
        assert_eq!(deferred.due(&subs, &[], &reduced, 3 * 60).len(), 0);
        let morning = deferred.due(&subs, &[], &reduced, 7 * 60);
        assert_eq!(morning.len(), 1);
        assert_eq!(morning[0].1, began);
        assert_eq!(
            deferred.due(&subs, &[], &reduced, 7 * 60 + 1).len(),
            0,
            "once"
        );

        // 23:00 it begins, 02:00 it ends: nothing in the morning.
        let mut deferred = Deferred::default();
        deferred.due(&subs, &[began], &reduced, 23 * 60);
        deferred.due(&subs, &[Event::ReductionEnded.into()], &free, 2 * 60);
        assert_eq!(deferred.due(&subs, &[], &free, 7 * 60).len(), 0);

        // Held, and no longer true by morning without an end being seen.
        let mut deferred = Deferred::default();
        deferred.due(
            &subs,
            &[Event::Failsafe.into()],
            &home(Some(true), None, &[]),
            0,
        );
        assert_eq!(deferred.due(&subs, &[], &free, 7 * 60).len(), 0);

        // A browser that unsubscribed is owed nothing.
        let mut deferred = Deferred::default();
        deferred.due(&subs, &[began], &reduced, 23 * 60);
        deferred.due(&[], &[], &reduced, 23 * 60 + 1);
        assert_eq!(deferred.due(&subs, &[], &reduced, 7 * 60).len(), 0);
    }

    #[test]
    fn the_local_minute_is_the_wall_clock_s() {
        // 16:00 UTC in November is 17:00 in Berlin; in July, 18:00.
        assert_eq!(local_minute(T0), 17 * 60);
        assert_eq!(local_minute(datetime!(2026-07-01 16:30 UTC)), 18 * 60 + 30);
    }

    #[test]
    fn only_a_push_service_over_https_is_an_endpoint() {
        let allow = Allow::default();
        for good in [
            "https://web.push.apple.com/QOe2abc",
            "https://fcm.googleapis.com/fcm/send/abc:def",
            "https://jmt17.google.com/fcm/send/abc:def",
            "https://updates.push.services.mozilla.com/wpush/v2/gAAAA",
            "https://db5p.notify.windows.com/w/?token=abc",
        ] {
            assert!(allow.audience(good).is_ok(), "{good}");
        }
        assert_eq!(
            allow
                .audience("https://fcm.googleapis.com/fcm/send/abc")
                .unwrap(),
            "https://fcm.googleapis.com",
            "the audience is the origin [RFC 8292 2]"
        );
        for bad in [
            "http://web.push.apple.com/abc",
            "https://127.0.0.1/abc",
            "https://192.168.1.1/admin",
            "https://[::1]/abc",
            "https://169.254.169.254/latest/meta-data",
            "https://hems.local/v1/pairing",
            "https://fcm.googleapis.com:8443/abc",
            "https://user:pw@fcm.googleapis.com/abc",
            "https://fcm.googleapis.com.evil.example/abc",
            "https://evilpush.apple.com/abc",
            "https://googleapis.com/abc",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert!(allow.audience(bad).is_err(), "{bad}");
        }
        assert!(
            allow
                .audience(&format!("https://web.push.apple.com/{}", "a".repeat(2048)))
                .is_err()
        );
    }

    // ── SC-001: a real request, to a push service that is not ours ───────────

    type Inbox = Arc<Mutex<Vec<(axum::http::HeaderMap, Vec<u8>)>>>;

    /// A push service on loopback: keeps what it is sent, answers `status`.
    async fn push_service(status: u16) -> (String, Inbox) {
        let inbox = Inbox::default();
        let app = axum::Router::new()
            .route(
                "/push/{id}",
                axum::routing::post(
                    move |axum::extract::State(inbox): axum::extract::State<Inbox>,
                          headers: axum::http::HeaderMap,
                          body: axum::body::Bytes| async move {
                        inbox.lock().await.push((headers, body.to_vec()));
                        axum::http::StatusCode::from_u16(status).unwrap()
                    },
                ),
            )
            .with_state(inbox.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (origin, inbox)
    }

    async fn sender(origin: &str) -> Push {
        let store = Arc::new(Mutex::new(crate::store::Store::in_memory().unwrap()));
        Push::open_with(store, Allow::Also(origin.to_owned()))
            .await
            .unwrap()
    }

    fn asked(endpoint: String, receiver: &Receiver) -> Asked {
        Asked {
            endpoint,
            keys: AskedKeys {
                p256dh: webpush::b64(&receiver.public),
                auth: webpush::b64(&receiver.auth),
            },
            lang: Some(Lang::De),
            events: None,
            quiet: None,
        }
    }

    #[tokio::test]
    async fn a_reduction_is_exactly_two_encrypted_notifications_only_the_browser_can_read() {
        let (origin, inbox) = push_service(201).await;
        let push = sender(&origin).await;
        let browser = Receiver::fresh();
        let page = std::env::var("EMS_TEST_NOTIFICATION_ORIGIN")
            .unwrap_or_else(|_| "https://hems.local".to_owned());
        push.subscribe(
            asked(format!("{origin}/push/one"), &browser),
            page.clone(),
            T0,
        )
        .await
        .expect("subscribed");

        // A reduction from 17:00 to 18:30 as the box sees it, tick by tick.
        let mut detector = Detector::default();
        let mut deferred = Deferred::default();
        let mut outcomes = Vec::new();
        let mut expected_ids = Vec::new();
        for minute in 0..=160 {
            let signals = home((60..150).contains(&minute).then_some(false), None, &[]);
            let now = at(minute);
            let changes = detector.changes(&signals, now);
            let events = detector.notifications(&changes);
            let subscriptions = push.subscriptions().await.unwrap();
            for (s, e) in deferred.due(&subscriptions, &events, &signals, local_minute(now)) {
                expected_ids.push(e.id.to_string());
                outcomes.push(push.send(s, e, now).await);
            }
        }
        assert_eq!(outcomes, [Outcome::Accepted, Outcome::Accepted]);

        let received = inbox.lock().await;
        assert_eq!(received.len(), 2, "exactly two");
        let stranger = Receiver::fresh();
        let mut documents = Vec::new();
        for ((headers, body), id) in received.iter().zip(expected_ids) {
            assert_eq!(headers["content-encoding"], "aes128gcm");
            assert_eq!(headers["ttl"], "1800");
            assert_eq!(headers["topic"], "ems-grid");
            assert!(
                !body.windows(4).any(|w| w == b"hems" || w == b"Netz"),
                "the push service carries ciphertext"
            );
            assert_eq!(stranger.open(body), None);
            let plain = browser.open(body).expect("the browser reads it");
            let document: serde_json::Value = serde_json::from_slice(&plain).unwrap();
            assert_eq!(document["web_push"], 8030);
            assert_eq!(document["notification"]["data"]["event_id"], id);
            assert_eq!(document["notification"]["navigate"], format!("{page}/#now"));
            documents.push(document);

            // Signed for this push service, by the key the browser subscribed with.
            let authorization = headers["authorization"].to_str().unwrap();
            let (token, key) = authorization
                .strip_prefix("vapid t=")
                .and_then(|r| r.split_once(", k="))
                .unwrap();
            assert_eq!(key, push.overview().await.unwrap().key);
            let claims: serde_json::Value =
                serde_json::from_slice(&webpush::unb64(token.split('.').nth(1).unwrap()).unwrap())
                    .unwrap();
            assert_eq!(claims["aud"], origin);
            assert!(claims["exp"].as_i64().unwrap() - T0.unix_timestamp() <= 24 * 3600 + 160 * 60);
        }
        assert_eq!(
            documents
                .iter()
                .map(|doc| doc["notification"]["title"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                Event::ReductionBegan.sentence(Lang::De).0,
                Event::ReductionEnded.sentence(Lang::De).0
            ]
        );
        // The worker acceptance test consumes the two decrypted wire documents.
        if let Some(path) = std::env::var_os("EMS_TEST_NOTIFICATION_CAPTURE") {
            std::fs::write(path, serde_json::to_vec(&documents).unwrap()).unwrap();
        }
    }

    #[tokio::test]
    async fn a_subscription_the_push_service_no_longer_knows_is_forgotten() {
        let (origin, _inbox) = push_service(410).await;
        let push = sender(&origin).await;
        let endpoint = format!("{origin}/push/gone");
        push.subscribe(
            asked(endpoint.clone(), &Receiver::fresh()),
            String::new(),
            T0,
        )
        .await
        .unwrap();
        assert_eq!(push.test(&endpoint, T0).await.unwrap(), Some(Outcome::Gone));
        assert_eq!(push.overview().await.unwrap().subscriptions, []);
        assert_eq!(push.test(&endpoint, T0).await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_unreachable_or_unwilling_push_service_costs_nothing_and_keeps_the_subscription() {
        let (origin, _inbox) = push_service(403).await;
        let push = sender(&origin).await;
        let endpoint = format!("{origin}/push/x");
        push.subscribe(
            asked(endpoint.clone(), &Receiver::fresh()),
            String::new(),
            T0,
        )
        .await
        .unwrap();
        assert_eq!(
            push.test(&endpoint, T0).await.unwrap(),
            Some(Outcome::Refused(403))
        );

        // Nothing listens here: dropped, not queued, and not an error.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let push = sender(&dead).await;
        let endpoint = format!("{dead}/push/x");
        push.subscribe(
            asked(endpoint.clone(), &Receiver::fresh()),
            String::new(),
            T0,
        )
        .await
        .unwrap();
        assert_eq!(
            push.test(&endpoint, T0).await.unwrap(),
            Some(Outcome::Unreachable)
        );
        assert_eq!(push.overview().await.unwrap().subscriptions.len(), 1);
    }

    #[tokio::test]
    async fn a_subscription_is_checked_before_it_is_kept() {
        let (origin, _inbox) = push_service(201).await;
        let push = sender(&origin).await;
        let browser = Receiver::fresh();
        let good = || asked(format!("{origin}/push/a"), &browser);

        let elsewhere = Asked {
            endpoint: "https://192.168.1.1/cgi-bin/reboot".into(),
            ..good()
        };
        assert!(matches!(
            push.subscribe(elsewhere, String::new(), T0).await,
            Err(Refused::Endpoint(_))
        ));
        let mut bad_key = good();
        bad_key.keys.p256dh = webpush::b64(&[4; 65]);
        assert!(matches!(
            push.subscribe(bad_key, String::new(), T0).await,
            Err(Refused::Keys(webpush::Error::Key))
        ));
        let mut bad_auth = good();
        bad_auth.keys.auth = "c2hvcnQ".into();
        assert!(matches!(
            push.subscribe(bad_auth, String::new(), T0).await,
            Err(Refused::Keys(webpush::Error::Auth))
        ));
        for quiet in [(1440, 0), (0, 1440), (300, 300)] {
            let mut bad_quiet = good();
            bad_quiet.quiet = Some(Quiet {
                from_minute: quiet.0,
                to_minute: quiet.1,
            });
            assert!(matches!(
                push.subscribe(bad_quiet, String::new(), T0).await,
                Err(Refused::Quiet)
            ));
        }
        assert_eq!(push.overview().await.unwrap().subscriptions, []);

        // The cap, and changing one already there.
        for n in 0..MAX_SUBSCRIPTIONS {
            push.subscribe(
                asked(format!("{origin}/push/{n}"), &browser),
                String::new(),
                T0,
            )
            .await
            .unwrap();
        }
        assert!(matches!(
            push.subscribe(
                asked(format!("{origin}/push/more"), &browser),
                String::new(),
                T0
            )
            .await,
            Err(Refused::Full)
        ));
        let mut change = asked(format!("{origin}/push/0"), &browser);
        change.events = Some([Kind::Goal].into_iter().collect());
        change.lang = Some(Lang::En);
        let view = push.subscribe(change, String::new(), T0).await.unwrap();
        assert_eq!(view.events.len(), 1);
        let overview = push.overview().await.unwrap();
        assert_eq!(overview.subscriptions.len() as u64, MAX_SUBSCRIPTIONS);
        // The page is never handed a browser's keys back.
        let shown = serde_json::to_string(&overview).unwrap();
        assert!(!shown.contains(&webpush::b64(&browser.auth)));
        assert!(!shown.contains("p256dh"));
    }

    #[tokio::test]
    async fn the_signing_key_survives_a_restart() {
        let store = Arc::new(Mutex::new(crate::store::Store::in_memory().unwrap()));
        let first = Push::open(store.clone()).await.unwrap();
        let second = Push::open(store).await.unwrap();
        assert_eq!(
            first.overview().await.unwrap().key,
            second.overview().await.unwrap().key,
            "a browser's subscription is bound to it"
        );
    }
}
