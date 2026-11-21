//! The box's HTTP API as a contract.
//!
//! One table — [`routes`] — says what `hemsd run` serves, who may ask, and
//! which type answers. [`document`] turns it into OpenAPI 3.1, with every
//! schema derived from the type the handler itself returns, so a field that
//! changes there changes here. `cargo xtask check-api` holds the checked-in
//! `openapi.json` and the reference page to this module's output.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::{Value, json};

use crate::home::{CarGoal, GoalAnswer, Home, TankAnswer, TankGoal};
use crate::runtime::{access, api, handover, push};

/// Whose credential a route takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    /// Nobody's: probes, and signing in.
    Anyone,
    /// The household's token or its session cookie.
    Household,
    /// The household's token itself; the session cookie is refused.
    HouseholdToken,
    /// The household's, or a connected energy manager's.
    HouseholdOrManager,
}

impl Who {
    /// How the reference page names it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Anyone => "none",
            Self::Household => "household",
            Self::HouseholdToken => "household token",
            Self::HouseholdOrManager => "household or manager",
        }
    }

    fn security(self) -> Value {
        match self {
            Self::Anyone => json!([]),
            Self::Household => json!([{ "household": [] }, { "session": [] }]),
            Self::HouseholdToken => json!([{ "household": [] }]),
            Self::HouseholdOrManager => {
                json!([{ "household": [] }, { "session": [] }, { "manager": [] }])
            }
        }
    }
}

/// A schema, made on demand so every type lands in one set of definitions.
pub type Shape = fn(&mut SchemaGenerator) -> Schema;

/// What a response carries.
#[derive(Clone, Copy)]
pub enum Body {
    /// Nothing.
    Empty,
    /// JSON of this shape.
    Json(Shape),
    /// A sentence saying why, as `text/plain`.
    Text,
    /// `image/svg+xml`.
    Svg,
    /// `text/csv`.
    Csv,
    /// `application/jose`: a JWS in compact serialisation whose payload is
    /// JSON of this shape.
    Jws(Shape),
}

/// One operation.
pub struct Route {
    /// `GET`, `PUT`, `POST` or `DELETE`.
    pub method: &'static str,
    /// The path, with `{parameters}` as the router spells them.
    pub path: &'static str,
    /// What it does, in one line.
    pub summary: &'static str,
    /// Whose credential it takes.
    pub who: Who,
    /// Query parameters: name and meaning. All optional.
    pub query: &'static [(&'static str, &'static str)],
    /// The JSON body it takes.
    pub body: Option<Shape>,
    /// The statuses its handler returns, each with what it means.
    pub answers: &'static [(u16, &'static str, Body)],
}

fn of<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    generator.subschema_for::<T>()
}

fn goal_body(generator: &mut SchemaGenerator) -> Schema {
    let (car, tank) = (of::<CarGoal>(generator), of::<TankGoal>(generator));
    json_schema!({ "oneOf": [car, tank] })
}

fn goal_answer(generator: &mut SchemaGenerator) -> Schema {
    let (car, tank) = (of::<GoalAnswer>(generator), of::<TankAnswer>(generator));
    json_schema!({ "oneOf": [car, tank, { "type": "null" }] })
}

fn readiness(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "object",
        "required": ["ready", "probes"],
        "properties": {
            "ready": { "type": "boolean" },
            "probes": { "type": "object", "description": "One entry per named dependency." }
        }
    })
}

fn openapi_document(_: &mut SchemaGenerator) -> Schema {
    schemars::json_schema!({
        "type": "object",
        "description": "An OpenAPI 3.1 document",
        "required": ["openapi", "info", "paths"]
    })
}

fn version(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "object", "required": ["name", "version"], "properties": { "name": { "type": "string" }, "version": { "type": "string" } } })
}

/// `boost`, `pause` or `away`: [`hems_core::setpoint::UserOverride`], which
/// lives in a crate that does not know about schemas. A test holds the list to
/// what the type serialises as.
pub fn user_override(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "string", "enum": ["boost", "pause", "away"] })
}

/// [`hems_forecast::Health`]: tagged by `verdict`, with the figures behind it.
pub fn roof_health(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": ["object", "null"],
        "required": ["verdict"],
        "properties": { "verdict": { "type": "string", "enum": ["learning", "healthy", "degraded"] } },
        "additionalProperties": { "type": "number" }
    })
}

/// What a SunSpec device says it is: [`hems_drv::modbus::probe::Identity`].
pub fn sunspec_identity(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": ["object", "null"],
        "required": ["base", "manufacturer", "model", "version", "serial"],
        "properties": {
            "base": { "type": "integer" }, "manufacturer": { "type": "string" }, "model": { "type": "string" },
            "version": { "type": "string" }, "serial": { "type": "string" }
        }
    })
}

const NO_STORE: &str = "This box keeps no store";
const WINDOW: &[(&str, &str)] = &[
    ("from", "first day, `YYYY-MM-DD`"),
    ("to", "last day, `YYYY-MM-DD`"),
];

/// Everything `hemsd run` serves, apart from the household page's own files.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "a table; splitting it would hide that it is one"
)]
pub fn routes() -> Vec<Route> {
    use Body::{Csv, Empty, Json, Jws, Svg, Text};
    use Who::{Anyone, Household, HouseholdOrManager, HouseholdToken};
    let r = |method, path, summary, who, query, body, answers| Route {
        method,
        path,
        summary,
        who,
        query,
        body,
        answers,
    };
    vec![
        r(
            "GET",
            "/livez",
            "Whether the process is alive",
            Anyone,
            &[],
            None,
            &[(200, "Alive", Empty), (503, "Not alive", Empty)],
        ),
        r(
            "GET",
            "/readyz",
            "Whether the box is ready to control",
            Anyone,
            &[],
            None,
            &[
                (200, "Ready", Json(readiness)),
                (503, "Not ready, and why", Json(readiness)),
            ],
        ),
        r(
            "GET",
            "/version",
            "The daemon's name and version",
            Anyone,
            &[],
            None,
            &[(200, "The version", Json(version))],
        ),
        r(
            "GET",
            "/openapi.json",
            "This contract: the OpenAPI 3.1 document the box was built with",
            Anyone,
            &[],
            None,
            &[(200, "The document", Json(openapi_document))],
        ),
        r(
            "GET",
            "/metrics",
            "Prometheus metrics",
            Anyone,
            &[],
            None,
            &[(200, "The metrics, in the Prometheus text format", Text)],
        ),
        r(
            "POST",
            "/v1/session",
            "Sign a browser in: exchange the household's token for a cookie",
            Anyone,
            &[],
            Some(of::<access::SignIn>),
            &[
                (204, "Signed in; the cookie is set", Empty),
                (401, "Not the household's token", Empty),
            ],
        ),
        r(
            "DELETE",
            "/v1/session",
            "Sign a browser out",
            Anyone,
            &[],
            None,
            &[(204, "The cookie is cleared", Empty)],
        ),
        r(
            "POST",
            "/v1/token",
            "Replace the household's token; the old one and every sign-in stop working",
            HouseholdToken,
            &[],
            None,
            &[
                (200, "The new token", Json(of::<api::NewToken>)),
                (409, "The token is set in the configuration", Text),
                (500, "It could not be kept", Text),
            ],
        ),
        r(
            "GET",
            "/v1/home",
            "The household page's document: now, the plan, goals, the last receipt",
            Household,
            &[],
            None,
            &[
                (200, "The document", Json(of::<Home>)),
                (404, "The box has no site to describe", Empty),
            ],
        ),
        r(
            "GET",
            "/v1/status",
            "What the box is doing and what it cannot see",
            HouseholdOrManager,
            &[],
            None,
            &[(200, "The status", Json(of::<api::StatusBody>))],
        ),
        r(
            "POST",
            "/v1/goals/{asset}/quote",
            "What a goal would cost, before it is set: the same plan solved with it",
            Household,
            &[],
            Some(goal_body),
            &[
                (
                    200,
                    "The verdict and the price",
                    Json(of::<crate::runtime::quote::Quote>),
                ),
                (400, "Out of range", Text),
                (404, "No such device", Text),
                (422, "The wrong goal for this kind of device", Text),
                (
                    429,
                    "A quote is already running, or one ended under a second ago",
                    Text,
                ),
                (
                    503,
                    "The box has not planned yet, or cannot see this device",
                    Text,
                ),
            ],
        ),
        r(
            "PUT",
            "/v1/goals/{asset}",
            "Ask for a charge or a temperature by a time; answered with a verdict",
            Household,
            &[],
            Some(goal_body),
            &[
                (
                    200,
                    "The verdict, or null before the planner has seen the device",
                    Json(goal_answer),
                ),
                (400, "Out of range", Text),
                (404, "No such device", Text),
                (422, "The wrong goal for this kind of device", Text),
                (500, "It could not be kept", Text),
            ],
        ),
        r(
            "DELETE",
            "/v1/goals/{asset}",
            "Withdraw a goal",
            Household,
            &[],
            None,
            &[
                (204, "Withdrawn", Empty),
                (404, "No such device, or no goal set", Text),
                (422, "This kind of device takes no goal", Text),
                (500, "It could not be kept", Text),
            ],
        ),
        r(
            "GET",
            "/v1/overrides",
            "The overrides in force",
            HouseholdOrManager,
            &[],
            None,
            &[(200, "The overrides", Json(of::<Vec<api::OverrideView>>))],
        ),
        r(
            "DELETE",
            "/v1/overrides",
            "Withdraw every override",
            HouseholdOrManager,
            &[],
            None,
            &[
                (204, "Withdrawn", Empty),
                (404, "No override was in force", Text),
            ],
        ),
        r(
            "PUT",
            "/v1/overrides/{asset}",
            "Boost, pause or mark away; the guard still narrows it",
            HouseholdOrManager,
            &[],
            Some(of::<api::OverrideBody>),
            &[
                (200, "The override as held", Json(of::<api::OverrideView>)),
                (400, "Not a device name", Empty),
            ],
        ),
        r(
            "DELETE",
            "/v1/overrides/{asset}",
            "Withdraw one override",
            HouseholdOrManager,
            &[],
            None,
            &[
                (204, "Withdrawn", Empty),
                (400, "Not a device name", Empty),
                (404, "None was set", Empty),
            ],
        ),
        r(
            "GET",
            "/v1/series/{point}",
            "The box's own measurements of `grid` or a device",
            Household,
            &[
                ("from", "start, RFC 3339; an hour before `to` by default"),
                ("to", "end, RFC 3339; now by default"),
            ],
            None,
            &[
                (200, "The readings", Json(of::<crate::series::Window>)),
                (400, "The window ends before it starts", Empty),
                (404, "This box keeps no series", Empty),
                (500, "It could not be read", Empty),
            ],
        ),
        r(
            "GET",
            "/v1/days",
            "The closed days the box keeps, at most ninety, and their sum",
            Household,
            WINDOW,
            None,
            &[
                (
                    200,
                    "The days, oldest first; a missing date is a day the box was off",
                    Json(of::<crate::month::Days>),
                ),
                (400, "Not a window", Empty),
                (404, NO_STORE, Empty),
                (500, "They could not be read", Empty),
            ],
        ),
        r(
            "GET",
            "/v1/days.csv",
            "The same days as CSV",
            Household,
            WINDOW,
            None,
            &[
                (200, "One row per day", Csv),
                (400, "Not a window", Empty),
                (404, NO_STORE, Empty),
                (500, "They could not be read", Empty),
            ],
        ),
        r(
            "GET",
            "/v1/pairing",
            "The control boxes this box trusts, and those asking",
            Household,
            &[],
            None,
            &[(200, "Trusted and waiting", Json(of::<api::PairingBody>))],
        ),
        r(
            "POST",
            "/v1/pairing",
            "Trust a control box by its SKI",
            Household,
            &[],
            Some(of::<api::Approval>),
            &[
                (
                    200,
                    "Trusted and waiting, afterwards",
                    Json(of::<api::PairingBody>),
                ),
                (400, "Not an SKI, or it could not be kept", Text),
                (404, "The box has no EEBUS identity", Text),
            ],
        ),
        r(
            "DELETE",
            "/v1/pairing/{ski}",
            "Stop trusting a control box",
            Household,
            &[],
            None,
            &[
                (204, "Forgotten", Empty),
                (400, "Not an SKI", Text),
                (404, "Not trusted, or the box has no EEBUS identity", Text),
                (500, "It could not be kept", Text),
            ],
        ),
        r(
            "POST",
            "/v1/pairing/{ski}/refuse",
            "Refuse a control box that is asking",
            Household,
            &[],
            None,
            &[
                (
                    200,
                    "Trusted and waiting, afterwards",
                    Json(of::<api::PairingBody>),
                ),
                (400, "Not an SKI", Text),
                (404, "The box has no EEBUS identity", Text),
            ],
        ),
        r(
            "GET",
            "/v1/trust",
            "Whether the box kept its identity, and whether its control box is heard",
            Household,
            &[],
            None,
            &[(200, "The check", Json(of::<handover::TrustCheck>))],
        ),
        r(
            "GET",
            "/v1/handover",
            "What the handover sheet prints: identities and the § 14a devices",
            Household,
            &[],
            None,
            &[
                (200, "The sheet", Json(of::<handover::Handover>)),
                (404, "The box has no site to describe", Empty),
            ],
        ),
        r(
            "GET",
            "/v1/commissioning",
            "The commissioning record, signed by the box: its identity, its control box, its § 14a devices",
            Household,
            &[(
                "reference",
                "the caller's own identifier for the work, echoed in the record: 1–64 printable ASCII characters, no space",
            )],
            None,
            &[
                (200, "The record", Jws(of::<handover::Record>)),
                (400, "Not a reference", Text),
                (404, "The box has no EEBUS identity to sign with", Text),
                (500, "It could not be signed", Text),
            ],
        ),
        r(
            "GET",
            "/v1/qr/{what}",
            "A QR code: `ski`, `fingerprint`, `commissioning`, or `signin` (token only, never the cookie)",
            Household,
            &[
                (
                    "origin",
                    "for `signin`: the address the code should open, `https://host[:port]`",
                ),
                (
                    "reference",
                    "for `commissioning`: as on `/v1/commissioning`",
                ),
            ],
            None,
            &[
                (200, "The code", Svg),
                (400, "Not an origin, or not a reference", Empty),
                (403, "`signin` asked for with the cookie", Empty),
                (404, "No such code, or no identity", Empty),
                (
                    413,
                    "`commissioning`: the record is longer than a code is scanned at; fetch it",
                    Text,
                ),
                (500, "It could not be drawn", Empty),
            ],
        ),
        r(
            "POST",
            "/v1/discover",
            "Look for SHIP nodes, and for SunSpec devices at named private addresses",
            Household,
            &[],
            Some(of::<handover::Ask>),
            &[
                (
                    200,
                    "What was found, set against the configuration",
                    Json(of::<handover::Discovered>),
                ),
                (
                    400,
                    "A host name, a public address, too many hosts, or another port",
                    Text,
                ),
            ],
        ),
        r(
            "GET",
            "/v1/managers",
            "The energy managers connected to this box",
            Household,
            &[],
            None,
            &[
                (200, "Their names", Json(of::<Vec<access::Manager>>)),
                (500, "They could not be read", Empty),
            ],
        ),
        r(
            "PUT",
            "/v1/managers/{name}",
            "Connect a manager; answers its token once",
            Household,
            &[],
            None,
            &[
                (200, "The name and the token", Json(of::<api::Connected>)),
                (400, "Not a name", Text),
                (500, "It could not be kept", Text),
            ],
        ),
        r(
            "DELETE",
            "/v1/managers/{name}",
            "Withdraw a manager's credential",
            Household,
            &[],
            None,
            &[
                (204, "Withdrawn", Empty),
                (404, "No such manager", Empty),
                (500, "It could not be kept", Text),
            ],
        ),
        r(
            "GET",
            "/v1/push",
            "The box's push key and the browsers subscribed",
            Household,
            &[],
            None,
            &[
                (200, "Key and subscriptions", Json(of::<push::Overview>)),
                (404, NO_STORE, Text),
                (500, "They could not be read", Text),
            ],
        ),
        r(
            "POST",
            "/v1/push",
            "Subscribe a browser, or change what it is told",
            Household,
            &[],
            Some(of::<push::Asked>),
            &[
                (200, "The subscription as kept", Json(of::<push::View>)),
                (
                    400,
                    "Not a known push service, bad keys or quiet hours",
                    Text,
                ),
                (404, NO_STORE, Text),
                (409, "Too many subscriptions", Text),
                (500, "It could not be kept", Text),
            ],
        ),
        r(
            "DELETE",
            "/v1/push",
            "Unsubscribe a browser",
            Household,
            &[],
            Some(of::<api::PushEndpoint>),
            &[
                (204, "Forgotten", Empty),
                (404, "No such subscription, or no store", Text),
                (500, "It could not be kept", Text),
            ],
        ),
        r(
            "POST",
            "/v1/push/test",
            "Send one test notification",
            Household,
            &[],
            Some(of::<api::PushEndpoint>),
            &[
                (204, "Accepted by the push service", Empty),
                (404, "No such subscription, or no store", Text),
                (410, "The subscription has ended and was forgotten", Text),
                (
                    502,
                    "The push service refused it or could not be reached",
                    Text,
                ),
                (500, "The subscription could not be read", Text),
            ],
        ),
        r(
            "GET",
            "/v1/subscriptions",
            "The systems this box tells about its events",
            Household,
            &[],
            None,
            &[(
                200,
                "Each, without its secret",
                Json(of::<Vec<crate::runtime::webhooks::View>>),
            )],
        ),
        r(
            "POST",
            "/v1/subscriptions",
            "Tell one more system: CloudEvents signed per Standard Webhooks",
            Household,
            &[],
            Some(of::<crate::runtime::webhooks::Asked>),
            &[
                (
                    201,
                    "Subscribed; the secret is shown this once",
                    Json(of::<crate::runtime::webhooks::Created>),
                ),
                (409, "Eight already", Text),
                (422, "A destination or type the box does not deliver", Text),
                (500, "It could not be kept", Text),
            ],
        ),
        r(
            "POST",
            "/v1/subscriptions/{id}/revoke",
            "Stop telling one, and forget what it was owed",
            Household,
            &[],
            None,
            &[(204, "Revoked", Empty), (404, "No such subscription", Text)],
        ),
        r(
            "POST",
            "/v1/subscriptions/{id}/test",
            "Send it a de.hems.test.ping now",
            Household,
            &[],
            None,
            &[
                (
                    200,
                    "What the receiver answered",
                    Json(of::<crate::runtime::webhooks::TestDelivery>),
                ),
                (404, "No such subscription", Text),
            ],
        ),
        r(
            "GET",
            "/v1/deliveries",
            "The last seven days of deliveries, newest first",
            Household,
            &[],
            None,
            &[(
                200,
                "Every attempt",
                Json(of::<Vec<crate::runtime::webhooks::Delivery>>),
            )],
        ),
        r(
            "GET",
            "/s2/{asset}",
            "S2 over WebSocket: a manager drives one device, narrowed by the guard",
            HouseholdOrManager,
            &[],
            None,
            &[
                (101, "Switching to the S2 WebSocket protocol", Empty),
                (404, "S2 is off, or no such device", Empty),
            ],
        ),
    ]
}

fn content(body: Body, generator: &mut SchemaGenerator) -> Option<Value> {
    Some(match body {
        Body::Empty => return None,
        Body::Json(shape) => json!({ "application/json": { "schema": shape(generator) } }),
        Body::Text => json!({ "text/plain": { "schema": { "type": "string" } } }),
        Body::Svg => json!({ "image/svg+xml": { "schema": { "type": "string" } } }),
        Body::Csv => json!({ "text/csv": { "schema": { "type": "string" } } }),
        Body::Jws(shape) => json!({ "application/jose": { "schema": {
            "type": "string",
            "description": "A JWS in compact serialisation (RFC 7515), ES256. \
                            `contentSchema` is its payload.",
            "contentMediaType": "application/json",
            "contentSchema": shape(generator),
        } } }),
    })
}

/// What the box sends to a subscribed system, one entry per event type: a
/// CloudEvent whose `data` has the schema below, signed per Standard Webhooks.
fn webhooks(generator: &mut SchemaGenerator) -> Value {
    use crate::runtime::webhooks as w;
    let data: [(&str, &str, Schema); 7] = [
        (
            hems_events::GRID_LPC_LIMIT_RECEIVED,
            "The network operator reduced the connection (§ 14a)",
            generator.subschema_for::<w::LimitReceived>(),
        ),
        (
            hems_events::GRID_LPC_LIMIT_RELEASED,
            "The reduction ended",
            generator.subschema_for::<w::LimitReleased>(),
        ),
        (
            hems_events::GRID_LPC_FAILSAFE_ENTERED,
            "The control box fell silent; the failsafe value applies",
            generator.subschema_for::<w::FailsafeEntered>(),
        ),
        (
            hems_events::SITE_GOAL_AT_RISK,
            "A goal turned tight or impossible",
            generator.subschema_for::<w::GoalAtRisk>(),
        ),
        (
            hems_events::DEVICE_DEVIATION_DETECTED,
            "A device has not done what it was asked for ten minutes",
            generator.subschema_for::<w::DeviationDetected>(),
        ),
        (
            hems_events::SITE_DAY_CLOSED,
            "A day closed: the receipt, in whole cents and Wh",
            generator.subschema_for::<crate::month::ClosedDay>(),
        ),
        (
            hems_events::TEST_PING,
            "Sent on request, to check a receiver",
            generator.subschema_for::<w::TestPing>(),
        ),
    ];
    let mut out = serde_json::Map::new();
    for (kind, summary, schema) in data {
        out.insert(kind.to_owned(), json!({ "post": {
            "summary": summary,
            "description": "A CloudEvent 1.0 in structured mode. Headers `webhook-id` (the event's `id`), \
                            `webhook-timestamp` and `webhook-signature` per Standard Webhooks, keyed with \
                            the subscription's `whsec_` secret. Answer `2xx`; anything else is retried for 24 h.",
            "requestBody": { "required": true, "content": { "application/cloudevents+json": { "schema": {
                "type": "object",
                "required": ["specversion", "id", "source", "type", "time", "data"],
                "properties": {
                    "specversion": { "const": "1.0" },
                    "id": { "type": "string" },
                    "source": { "type": "string" },
                    "type": { "const": kind },
                    "time": { "type": "string", "format": "date-time" },
                    "subject": { "type": "string" },
                    "datacontenttype": { "const": "application/json" },
                    "data": schema
                }
            } } } },
            "responses": { "200": { "description": "Received" } }
        } }));
    }
    out.into()
}

/// A refusal's body: one schema, named once, for every route.
fn problem(generator: &mut SchemaGenerator) -> Value {
    json!({ crate::runtime::problem::MEDIA_TYPE: {
        "schema": generator.subschema_for::<crate::runtime::problem::Problem>()
    } })
}

/// The OpenAPI 3.1 document.
///
/// # Panics
/// Never in practice: only if the table lists one method twice on one path.
#[must_use]
pub fn document() -> Value {
    let settings = schemars::generate::SchemaSettings::draft2020_12()
        .with(|s| s.definitions_path = "#/components/schemas/".into());
    let mut generator = settings.into_generator();
    let mut paths = serde_json::Map::new();
    for route in routes() {
        let mut responses = serde_json::Map::new();
        for &(status, meaning, body) in route.answers {
            let mut answer = json!({ "description": meaning });
            // Every refusal under `/v1/` is a problem document (`runtime::problem`),
            // whatever the handler wrote; the table says what it means.
            let content = if status >= 400 && route.path.starts_with("/v1/") {
                Some(problem(&mut generator))
            } else {
                content(body, &mut generator)
            };
            if let Some(content) = content {
                answer["content"] = content;
            }
            responses.insert(status.to_string(), answer);
        }
        if route.who != Who::Anyone {
            responses.insert(
                "401".into(),
                json!({ "description": "No credential, or not one this box issued",
                        "content": problem(&mut generator) }),
            );
            if route.who != Who::HouseholdOrManager {
                responses.entry("403").or_insert(json!({
                    "description": "A manager's credential, on a route that is the household's",
                    "content": problem(&mut generator)
                }));
            }
        }
        let mut parameters: Vec<Value> = route
            .path
            .split('/')
            .filter_map(|part| part.strip_prefix('{')?.strip_suffix('}'))
            .map(|name| json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" } }))
            .collect();
        parameters.extend(route.query.iter().map(|(name, meaning)| {
            json!({ "name": name, "in": "query", "description": meaning, "schema": { "type": "string" } })
        }));
        let mut operation = json!({
            "summary": route.summary,
            "security": route.who.security(),
            "responses": responses,
        });
        if !parameters.is_empty() {
            operation["parameters"] = parameters.into();
        }
        if let Some(shape) = route.body {
            operation["requestBody"] = json!({
                "required": true,
                "content": { "application/json": { "schema": shape(&mut generator) } }
            });
        }
        let previous = paths
            .entry(route.path)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("a path item is an object")
            .insert(route.method.to_lowercase(), operation);
        assert!(
            previous.is_none(),
            "{} {} is listed twice",
            route.method,
            route.path
        );
    }
    let webhooks = webhooks(&mut generator);
    let schemas: serde_json::Map<String, Value> =
        generator.take_definitions(true).into_iter().collect();
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "hemsd",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "The local API of a hems box. A request authenticated by the session cookie that \
                            changes anything must carry `Content-Type: application/json`."
        },
        "paths": paths,
        "webhooks": webhooks,
        "components": {
            "securitySchemes": {
                "household": { "type": "http", "scheme": "bearer", "description": "The household's token." },
                "session": { "type": "apiKey", "in": "cookie", "name": access::SESSION_COOKIE,
                             "description": "Set by `POST /v1/session`; `HttpOnly`, `SameSite=Strict`." },
                "manager": { "type": "http", "scheme": "bearer",
                             "description": "A token issued by `PUT /v1/managers/{name}`." }
            },
            "schemas": schemas
        }
    })
}

/// The document as it is checked in: pretty-printed, with a final newline.
///
/// # Panics
/// Never: a `Value` always serialises.
#[must_use]
pub fn json() -> String {
    let mut text = serde_json::to_string_pretty(&document()).expect("a JSON value serialises");
    text.push('\n');
    text
}

/// The reference page's table, one row per operation.
#[must_use]
pub fn markdown() -> String {
    use std::fmt::Write as _;
    let mut out =
        String::from("| Method | Path | Credential | What it does |\n|---|---|---|---|\n");
    for route in routes() {
        let _ = writeln!(
            out,
            "| `{}` | `{}` | {} | {} |",
            route.method,
            route.path,
            route.who.label(),
            route.summary
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every `(METHOD, path)` a `.route(…)` call in `source` mounts.
    fn mounted(source: &str) -> BTreeSet<(String, String)> {
        let production = source.split("#[cfg(test)]").next().unwrap_or(source);
        let mut out = BTreeSet::new();
        for call in production.split(".route(").skip(1) {
            // The call's own text: up to the parenthesis that closes it.
            let mut depth = 1usize;
            let end = call
                .char_indices()
                .find(|&(_, c)| {
                    match c {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                })
                .map_or(call.len(), |(i, _)| i);
            let call = &call[..end];
            let path = call.split('"').nth(1).expect("a route names its path");
            let handlers = &call[call.find(path).unwrap_or(0) + path.len()..];
            for method in ["get", "post", "put", "delete", "any"] {
                let named = handlers.match_indices(&format!("{method}(")).any(|(i, _)| {
                    !handlers[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_')
                });
                if named {
                    // An upgrade is asked for with GET, however it is mounted.
                    let method = if method == "any" { "get" } else { method };
                    out.insert((method.to_uppercase(), path.to_owned()));
                }
            }
        }
        out
    }

    #[test]
    fn the_table_lists_exactly_what_the_routers_mount() {
        let mut served = BTreeSet::new();
        for source in [
            include_str!("runtime/api.rs"),
            include_str!("runtime/s2.rs"),
            include_str!("../../../crates/hems-service/src/metrics.rs"),
        ] {
            served.extend(mounted(source));
        }
        // Only the routers `surfaces_over` and the shell merge, not test fixtures.
        let access = include_str!("runtime/access.rs");
        let session = &access[access
            .find("pub fn session_router")
            .expect("the session router")..];
        served.extend(mounted(&session[..session.find("\n}\n").expect("its end")]));
        let runtime = include_str!("runtime/mod.rs");
        let surfaces = &runtime[runtime.find("pub fn surfaces_over").expect("the surfaces")..];
        served.extend(mounted(
            &surfaces[..surfaces.find("\n}\n").expect("their end")],
        ));
        // The catch-all that turns an unknown path into a problem is not an
        // operation; it is the absence of one.
        served.retain(|(_, path)| !path.contains("{*"));
        let serve = include_str!("../../../crates/hems-service/src/serve.rs");
        let shared = &serve[serve.find("fn shared_routes").expect("the shared routes")..];
        served.extend(mounted(&shared[..shared.find("\n}\n").expect("their end")]));

        let listed: BTreeSet<(String, String)> = routes()
            .iter()
            .map(|r| (r.method.to_owned(), r.path.to_owned()))
            .collect();
        let unlisted: Vec<_> = served.difference(&listed).collect();
        let unserved: Vec<_> = listed.difference(&served).collect();
        assert!(
            unlisted.is_empty(),
            "served but not in the contract: {unlisted:?}"
        );
        assert!(
            unserved.is_empty(),
            "in the contract but not served: {unserved:?}"
        );
    }

    #[test]
    fn the_household_only_routes_are_the_ones_the_access_layer_guards() {
        for route in routes() {
            let guarded = access::is_household_only(route.path);
            let says = matches!(route.who, Who::Household | Who::HouseholdToken);
            if route.who != Who::Anyone {
                assert_eq!(guarded, says, "{} {}", route.method, route.path);
            }
        }
    }

    #[test]
    fn a_field_of_the_page_document_is_a_field_of_the_contract() {
        let doc = document();
        let schema = &doc["components"]["schemas"]["Home"]["properties"];
        // Written out, so a new field does not compile until it is here too.
        let sample = serde_json::to_value(Home {
            site: String::new(),
            at: None,
            demo: false,
            at_minute: Some(0),
            grid_kw: None,
            price_ct: None,
            reduction: None,
            devices: Vec::new(),
            plan: Vec::new(),
            ahead: Some(crate::home::Ahead {
                from: String::new(),
                until: String::new(),
                bill_eur: 0.0,
            }),
            worth: Some(crate::home::Worth {
                battery_ct: 0.0,
                export_ct: 0.0,
            }),
            receipt: None,
            goal: None,
            tank_goal: None,
            ski: None,
            fingerprint: None,
            https_fingerprint: None,
            roof: None,
            levies_year: Some(2027),
        })
        .expect("a document");
        let fields = sample.as_object().expect("an object");
        assert!(!fields.is_empty());
        for field in fields.keys() {
            assert!(
                schema.get(field).is_some(),
                "`{field}` is served but not in the contract"
            );
        }
    }

    #[test]
    fn the_hand_written_shapes_are_what_the_types_serialise_as() {
        use hems_core::setpoint::UserOverride as U;
        let mut generator = SchemaGenerator::default();
        let listed = user_override(&mut generator).to_value()["enum"].clone();
        let real: Vec<Value> = [U::Boost, U::Pause, U::Away]
            .iter()
            .map(|u| serde_json::to_value(u).expect("a word"))
            .collect();
        assert_eq!(listed, Value::from(real));

        let health =
            serde_json::to_value(hems_forecast::Health::Learning { days: 3 }).expect("a verdict");
        assert_eq!(health["verdict"], "learning");
    }

    #[test]
    fn the_contract_carries_no_credential() {
        let text = json();
        assert!(
            !text.contains("Bearer "),
            "an example token would be mistaken for a real one"
        );
        assert!(!text.contains("\"example\""));
    }

    #[test]
    fn every_reference_resolves() {
        let doc = document();
        let text = doc.to_string();
        for reference in text.split("\"#/components/schemas/").skip(1) {
            let name = reference.split('"').next().expect("a name");
            assert!(
                doc["components"]["schemas"].get(name).is_some(),
                "`{name}` is referenced but not defined"
            );
        }
    }
}
