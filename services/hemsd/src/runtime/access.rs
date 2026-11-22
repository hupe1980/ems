//! Who may ask this box anything.
//!
//! The box holds a household's own electricity: `/v1/status` says what every
//! device is doing right now, `/v1/series` is the Data Act's local API over the
//! one-second history, `/v1/overrides` decides what the arbiter wants, and
//! `/s2/{asset}` lets an external energy manager drive the house. Every one of
//! them authenticates its caller through `ems_service::auth`, the same model
//! the fleet daemons use (D108, D112).
//!
//! # A token the box issues itself
//!
//! Not a setting with a default, which is the shape that ships insecure: a
//! default token is a published token, and a box that *refused to start* without
//! one would be a box an installer works around by inventing a weak one. So the
//! box generates its own on first start, from the operating system's entropy,
//! and keeps it in its own store beside the EEBUS key — for the same reason the
//! SKI lives there (`runtime::ship`): an installer reads it off a screen once,
//! and a value that changed on every boot would make that a step they had to
//! repeat.
//!
//! It is printed at start-up next to the SKI, which is the other credential a
//! commissioning visit has to carry away.
//!
//! **A box with no store cannot keep one**, and then the token is fresh every
//! run and says so. That is honest rather than convenient: `store_path = None`
//! is the demonstration mode, and a demonstration whose credential survived a
//! restart would be keeping state it has been told not to keep.
//!
//! A deployment that provisions credentials of its own sets `[api] token`
//! instead, and then nothing is generated and nothing is stored.
//!
//! # What it is not
//!
//! It is **not** a second authorisation model. The token resolves to the same
//! `ems_service::auth::Authority` a fleet service would issue — `box_at(site)`,
//! the household's own — so the capability names, the attenuation rules and the
//! constant-time comparison are the workspace's one implementation rather than a
//! second opinion written for the edge.
//!
//! Nor does it protect the **health surface**. `/livez`, `/readyz` and
//! `/metrics` stay open, because an orchestrator that has to hold a household's
//! credential in order to restart a crashed box is an orchestrator that will be
//! given one too widely. They carry no household data — that is the property
//! that makes it safe, and it is `ems_service`'s to keep.

use std::sync::Arc;

use ems_service::auth::{Authority, Capabilities, Credentials, SiteScope, bearer};
use ems_service::config::Secret;
use tokio::sync::RwLock;

use crate::store::Store;

/// What a connected energy manager may do with the credential the box issued it.
///
/// The household's own set **less the Data Act export**: an aggregator drives
/// devices, and Article 4 of Regulation (EU) 2023/2854 is a right of the *user*
/// rather than of whoever the user has asked to optimise for them. A manager
/// that could take the one-second series could reconstruct when the household
/// showered, cooked and went away.
///
/// Narrowing it further — to the S2 routes alone — wants a capability of its own
/// (`hems.flex.drive`) rather than a special case here; `Capabilities` attenuate
/// by containment, so that is a new pattern rather than a new mechanism.
fn manager_capabilities() -> Capabilities {
    Capabilities::of(ems_service::auth::ALL_CAPABILITIES.iter().copied())
        .attenuate(&Capabilities::of([
            ems_service::auth::RECORD_READ,
            ems_service::auth::RECORD_WRITE,
        ]))
        .unwrap_or_else(|_| Capabilities::none())
}

/// How the box's own surfaces are reached.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ApiSettings {
    /// The bearer token that opens `/v1/*` and `/s2/*`.
    ///
    /// Absent — the ordinary case — means the box issues one itself, keeps it in
    /// its own store and prints it at start-up. Set it where something else
    /// already provisions credentials for this fleet, and then nothing is
    /// generated and nothing is stored.
    ///
    /// It is a [`Secret`], so `env:HEMS_API_TOKEN` and `file:/run/secrets/…`
    /// work here as they do everywhere else in this workspace — a token written
    /// literally into a configuration file is a token in a backup.
    pub token: Option<Secret>,
    /// Serve the box's surfaces over HTTPS with a certificate it issues and
    /// keeps itself.
    ///
    /// Absent means: on, exactly when `[service] listen` is not a loopback
    /// address. A token sent in clear across a household network is a token
    /// any device on it can read, and an S2 manager will not send one at all
    /// (S2 Connect: S2 must not travel unencrypted).
    pub tls: Option<bool>,
    /// Serve plain HTTP on a non-loopback address anyway.
    ///
    /// A written decision rather than a default, and logged at every start:
    /// for a box behind a proxy that terminates TLS itself, and for nothing
    /// else.
    pub insecure_lan: bool,
}

impl ApiSettings {
    /// Whether the box serves HTTPS when it listens on `listen`.
    ///
    /// # Errors
    /// Where it would listen on a non-loopback address in clear and
    /// `insecure_lan` does not say so.
    pub fn tls_on(&self, listen: std::net::SocketAddr) -> Result<bool, String> {
        let tls = self.tls.unwrap_or(!listen.ip().is_loopback());
        if !tls && !listen.ip().is_loopback() && !self.insecure_lan {
            return Err(format!(
                "`[service] listen = {listen}` serves the household's token in clear on its \
                 network; leave `[api] tls` on, or write `insecure_lan = true` if a proxy \
                 in front terminates TLS"
            ));
        }
        Ok(tls)
    }
}

/// The credentials this box answers to, and the household it answers about.
///
/// Two kinds. The household's own token — the box's until the household
/// replaces it ([`LocalAccess::replace_token`]) — and one
/// per **energy manager** it has connected — issued, listed and withdrawn while
/// the box runs, which is why the set is behind a lock rather than built once.
#[derive(Clone)]
pub struct LocalAccess {
    credentials: Arc<RwLock<Credentials>>,
    site: String,
    /// The household's own token, kept so the set can be **rebuilt** when a
    /// manager is withdrawn. `Credentials` is deliberately append-only.
    ///
    /// Shared and replaceable: the household can issue itself a new one
    /// ([`LocalAccess::replace_token`]), and every clone of this set has to
    /// answer to the new one at once.
    own: Arc<std::sync::RwLock<String>>,
    /// Whether the box issued the token itself. One a deployment configured is
    /// that deployment's to rotate, and is not replaced from here.
    issued_here: bool,
    store: Option<Arc<tokio::sync::Mutex<Store>>>,
}

/// An energy manager this household has connected.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct Manager {
    /// What the household calls it.
    pub name: String,
}

impl LocalAccess {
    /// Resolve the box's own credential: configured, remembered, or issued.
    ///
    /// # Errors
    /// When a configured secret names an environment variable that is not set
    /// or a file that cannot be read, or when the store refuses the write.
    pub fn resolve(
        settings: &ApiSettings,
        site: &str,
        store: Option<&Store>,
    ) -> anyhow::Result<(Self, Issued)> {
        let (token, issued) = match (&settings.token, store) {
            // A deployment provisioned one, so nothing is generated and nothing
            // is stored.
            (Some(secret), _) => (secret.resolve_from_process()?, How::Configured),
            // The box issued one before and remembered it — the ordinary case
            // after the first start.
            (None, Some(store)) => {
                if let Some(token) = store.api_token()? {
                    (token, How::Remembered)
                } else {
                    let token = generate();
                    store.put_api_token(&token)?;
                    (token, How::Fresh)
                }
            }
            // Nothing to keep it in, so it lasts as long as the process. See the
            // module note: that is what `store_path = None` means.
            (None, None) => (generate(), How::Ephemeral),
        };
        // One token, used for both: the box has to answer to exactly what it
        // prints, and building the credential and the announcement from separate
        // values is a box nobody can log in to.
        let mut access = Self::new(site, &token);
        access.issued_here = issued != How::Configured;
        Ok((access, Issued { token, how: issued }))
    }

    /// A credential set built in code, for a test.
    #[must_use]
    pub fn for_testing(site: &str, token: &str) -> Self {
        Self::new(site, token)
    }

    fn new(site: &str, token: &str) -> Self {
        Self {
            credentials: Arc::new(RwLock::new(Credentials::default().with_site(site, token))),
            site: site.to_owned(),
            own: Arc::new(std::sync::RwLock::new(token.to_owned())),
            issued_here: true,
            store: None,
        }
    }

    /// Keep the store, so a manager connected on a running box survives a
    /// restart — the same reason the EEBUS trust store is kept (D102).
    #[must_use]
    pub fn remembering(mut self, store: Option<Arc<tokio::sync::Mutex<Store>>>) -> Self {
        self.store = store;
        self
    }

    /// Load the managers this household connected earlier.
    ///
    /// # Errors
    /// [`StoreError`](crate::store::StoreError) where the read fails.
    pub async fn restore(&self) -> Result<usize, crate::store::StoreError> {
        let Some(store) = &self.store else {
            return Ok(0);
        };
        let managers = store.lock().await.managers()?;
        let mut credentials = self.credentials.write().await;
        for (name, token) in &managers {
            *credentials = credentials.clone().with_authority(
                Authority::new(
                    format!("cem:{name}"),
                    manager_capabilities(),
                    SiteScope::One(self.site.clone()),
                ),
                token,
            );
        }
        Ok(managers.len())
    }

    /// Connect an energy manager, and return the credential it presents.
    ///
    /// Replaces the credential of a manager already connected under that name,
    /// which is how one is **rotated**: the household names the same manager
    /// again and the old token stops working on the next request.
    ///
    /// # Errors
    /// [`ManagerError::Name`] for a name outside [`is_manager_name`], and
    /// [`ManagerError::Store`] where the write fails.
    pub async fn connect_manager(&self, name: &str) -> Result<String, ManagerError> {
        if !is_manager_name(name) {
            return Err(ManagerError::Name(name.escape_debug().to_string()));
        }
        let token = generate();
        if let Some(store) = &self.store {
            store.lock().await.put_manager(name, &token)?;
        }
        let mut credentials = self.credentials.write().await;
        *credentials = credentials.clone().with_authority(
            Authority::new(
                format!("cem:{name}"),
                manager_capabilities(),
                SiteScope::One(self.site.clone()),
            ),
            &token,
        );
        Ok(token)
    }

    /// Withdraw one. Returns whether it was connected.
    ///
    /// The credential stops working immediately rather than at the end of some
    /// session: a household that has decided a manager should stop driving its
    /// house has not decided it may finish the quarter hour first.
    ///
    /// # Errors
    /// [`StoreError`](crate::store::StoreError) where the write fails.
    pub async fn forget_manager(&self, name: &str) -> Result<bool, crate::store::StoreError> {
        let existed = match &self.store {
            Some(store) => store.lock().await.forget_manager(name)?,
            None => false,
        };
        // Rebuilt from the store rather than removed in place, because
        // `Credentials` is deliberately append-only: it is a set of things that
        // are allowed, and a type that could quietly drop one is a type where a
        // revocation can go missing.
        let rebuilt = Credentials::default().with_site(&self.site, self.household_token());
        let mut credentials = self.credentials.write().await;
        *credentials = rebuilt;
        drop(credentials);
        self.restore().await?;
        Ok(existed)
    }

    /// Every manager currently connected.
    ///
    /// # Errors
    /// [`StoreError`](crate::store::StoreError) where the read fails.
    pub async fn managers(&self) -> Result<Vec<Manager>, crate::store::StoreError> {
        let Some(store) = &self.store else {
            return Ok(Vec::new());
        };
        Ok(store
            .lock()
            .await
            .managers()?
            .into_iter()
            .map(|(name, _)| Manager { name })
            .collect())
    }

    /// Whether this request may act on this household.
    ///
    /// One answer for every failure — absent, malformed, unknown — because which
    /// it was is an operational fact for whoever runs the box and a probing aid
    /// for anybody else.
    pub async fn authority(&self, header: Option<&str>) -> Option<Authority> {
        let token = bearer(header)?;
        self.credentials.read().await.authority_of(token)
    }

    /// The authority a bare token resolves to — the household page's session
    /// cookie carries the token itself.
    pub async fn authority_of_token(&self, token: &str) -> Option<Authority> {
        self.credentials.read().await.authority_of(token)
    }

    /// The household's own token, for the sign-in code on the handover sheet.
    ///
    /// Crate-private, and read by exactly one handler, which answers only a
    /// caller that has just presented this same token — so it tells nobody
    /// anything they did not already hold (`api::qr`).
    pub(crate) fn household_token(&self) -> String {
        self.own
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Issue the household a new token and stop answering to the old one.
    ///
    /// What makes a printed handover sheet safe to lose: the sheet carries the
    /// token, and until this existed the only way to make a lost one worthless
    /// was a factory reset. Every session cookie and every sign-in code made
    /// before stops working on the next request. The managers the household
    /// connected keep their own credentials — they never held this one.
    ///
    /// # Errors
    /// [`ReplaceError::Configured`] where `[api] token` provisions the
    /// credential — that deployment rotates it where it keeps it — and
    /// [`ReplaceError::Store`] where the new one could not be written down, in
    /// which case the old one still stands.
    pub async fn replace_token(&self) -> Result<String, ReplaceError> {
        if !self.issued_here {
            return Err(ReplaceError::Configured);
        }
        let token = generate();
        // Written down first: a token the box answers to and has not kept is
        // one a power cut turns into a lock-out.
        if let Some(store) = &self.store {
            store.lock().await.put_api_token(&token)?;
        }
        self.own
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone_from(&token);
        let rebuilt = Credentials::default().with_site(&self.site, &token);
        *self.credentials.write().await = rebuilt;
        self.restore().await?;
        Ok(token)
    }

    /// The household this box is.
    #[must_use]
    pub fn site(&self) -> &str {
        &self.site
    }
}

/// Refuse any request that does not carry this box's own token.
///
/// **One layer over the whole router, not a check in each handler**, and that is
/// the shape rather than the convenience. `obsd` served its fleet summary — which
/// names every household that failed to respect a network operator's reduction —
/// to any valid credential, because four call sites each spelled the test
/// themselves and one spelled it wrong (D112). A route added to this box
/// tomorrow cannot forget a layer it never had to remember.
///
/// Every failure is one status. Which it was — no header, a malformed one, a
/// token nobody issued — is an operational fact for whoever runs the box, and a
/// probing aid for anybody else.
///
/// # Errors
/// [`axum::http::StatusCode::UNAUTHORIZED`] when the request carries no
/// credential this box issued.
pub async fn require_token(
    axum::extract::State(access): axum::extract::State<LocalAccess>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let presented = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let site = access.site().to_owned();
    let authority = match presented {
        Some(_) => access.authority(presented).await,
        // The household page signs in once and then presents the token as an
        // HttpOnly cookie, which no script on the page can read. A cookie is
        // sent by the browser on its own, so a write that relies on one must
        // also carry a JSON body type — which a cross-site form cannot send —
        // on top of the cookie's own SameSite=Strict.
        None => match session_token(request.headers()) {
            Some(token) => {
                let writes = !matches!(
                    *request.method(),
                    axum::http::Method::GET | axum::http::Method::HEAD
                );
                let json = request
                    .headers()
                    .get(axum::http::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.starts_with("application/json"));
                if writes && !json {
                    return Err(axum::http::StatusCode::UNAUTHORIZED);
                }
                access.authority_of_token(token).await
            }
            None => None,
        },
    };
    let Some(authority) = authority else {
        return Err(axum::http::StatusCode::UNAUTHORIZED);
    };
    if !authority.may_read(&site) {
        return Err(axum::http::StatusCode::UNAUTHORIZED);
    }
    // The Data Act export is the household's own, not its manager's: Article 4
    // of Regulation (EU) 2023/2854 is a right of the *user*, and the one-second
    // series says when they showered, cooked and went away. It is the one
    // capability a manager's credential does not carry, and this is where the
    // difference is enforced rather than merely granted.
    //
    // The same line holds for the household's own **decisions** about its box:
    // which control box it obeys, which managers hold a credential, and what the
    // car should reach by when. A manager that could approve a Steuerbox could
    // put a device of its own in the network operator's place; one that could
    // forget the household's could stop the box following § 14a; one that could
    // connect managers could mint credentials. A manager drives over S2.
    let path = request.uri().path();
    if is_household_only(path) && !authority.may_read_everything(&site) {
        return Err(axum::http::StatusCode::FORBIDDEN);
    }
    // Handed on, so whatever the request then does can say **who** did it: a
    // manager's instruction is attributed to `cem:<name>`, the household's own
    // to `site:<id>`, and an audit line that said "the box" would answer
    // nothing a household asks.
    let mut request = request;
    request.extensions_mut().insert(authority);
    Ok(next.run(request).await)
}

/// Whether `name` may name an energy manager: 1–64 characters of ASCII letters,
/// digits, `-`, `_` and `.`, starting with a letter or a digit.
///
/// Narrow on purpose. The name becomes the subject `cem:<name>` every
/// instruction the manager sends is logged under, so a name carrying a newline,
/// a space or an `=` could write a line into the household's audit trail that
/// looks like somebody else's.
#[must_use]
pub fn is_manager_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Why the household's token could not be replaced.
#[derive(Debug, thiserror::Error)]
pub enum ReplaceError {
    /// `[api] token` provisions it, so it is not this box's to replace.
    #[error("this box's token is set by `[api] token`; replace it where that is kept and restart")]
    Configured,
    /// The store refused the write.
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
}

/// Why a manager could not be connected.
#[derive(Debug, thiserror::Error)]
pub enum ManagerError {
    /// The name is not one [`is_manager_name`] accepts.
    #[error(
        "`{0}` cannot name a manager: use 1–64 letters, digits, `-`, `_` or `.`, starting with a letter or a digit"
    )]
    Name(String),
    /// The store refused the write.
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
}

/// Whether `path` is one only the household's own credential reaches.
#[must_use]
pub fn is_household_only(path: &str) -> bool {
    HOUSEHOLD_ONLY.iter().any(|p| path.starts_with(p))
}

/// Routes only the household's own credential reaches — its data, and its
/// decisions about who may act on the box.
const HOUSEHOLD_ONLY: &[&str] = &[
    "/v1/series",
    "/v1/pairing",
    "/v1/managers",
    "/v1/goals",
    // The page's own document carries the goals — when the car leaves, when
    // the water is wanted — and the day's receipt. A manager reads `/v1/status`.
    "/v1/home",
    "/v1/days",
    // Commissioning: the codes and the sheet carry the box's identities and one
    // of them the household's own credential, discovery opens sockets from the
    // box's place on the network, and the trust check says who the box obeys.
    "/v1/qr",
    "/v1/handover",
    // The signed record names the control box and every device; it is the
    // household's to hand to whoever it ordered the work from.
    "/v1/commissioning",
    "/v1/discover",
    "/v1/trust",
    // Replacing the household's token is the household's, and nobody else's.
    "/v1/token",
    // Notifications: who is told what this house's network operator is doing,
    // and where the box sends it, is the household's to decide. A manager that
    // could subscribe would be told when the household's car will not be ready.
    "/v1/push",
    // Events out, for the same reason: which systems hear about this house is
    // the household's decision. An energy manager is added by the household
    // like anyone else, never by itself.
    "/v1/subscriptions",
    "/v1/deliveries",
];

/// The cookie the household page signs in with.
pub const SESSION_COOKIE: &str = "hems_session";

/// The token in this request's session cookie, where there is one.
fn session_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(name, value)| (name == SESSION_COOKIE && !value.is_empty()).then_some(value))
}

/// What the page sends to sign in.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SignIn {
    /// The household's token, from the handover sheet or the start-up log.
    pub token: String,
}

/// Sign the household page in and out.
///
/// Outside [`require_token`] by necessity — it is how a browser *gets* the
/// credential that layer asks for — and it grants nothing the token did not
/// already: it checks the token and hands it back as a cookie the page's own
/// scripts cannot read. Only the **household's** own credential signs in; a
/// manager's drives devices and has no business reading the household's life.
/// `secure` adds the `Secure` attribute, which a box serving TLS sets.
pub fn session_router(access: LocalAccess, secure: bool) -> axum::Router {
    use axum::routing::post;
    axum::Router::new()
        .route(
            "/v1/session",
            post(
                move |axum::extract::State(access): axum::extract::State<LocalAccess>,
                      axum::Json(body): axum::Json<SignIn>| async move {
                    let site = access.site().to_owned();
                    let allowed = access
                        .authority_of_token(body.token.trim())
                        .await
                        .is_some_and(|a| a.may_read_everything(&site));
                    if !allowed {
                        return Err(axum::http::StatusCode::UNAUTHORIZED);
                    }
                    let cookie = format!(
                        "{SESSION_COOKIE}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=31536000{}",
                        body.token.trim(),
                        if secure { "; Secure" } else { "" }
                    );
                    Ok((
                        axum::http::StatusCode::NO_CONTENT,
                        [(axum::http::header::SET_COOKIE, cookie)],
                    ))
                },
            )
            .delete(move || async move {
                (
                    axum::http::StatusCode::NO_CONTENT,
                    [(
                        axum::http::header::SET_COOKIE,
                        format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
                    )],
                )
            }),
        )
        .with_state(access)
}

/// Where the token came from, so start-up can say the right thing about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Issued {
    /// The token itself, for the line an installer reads.
    pub token: String,
    /// How the box came by it.
    pub how: How,
}

impl Issued {
    /// The line to print at start-up, or `None` where the operator already has
    /// the token because they configured it.
    #[must_use]
    pub fn announcement(&self) -> Option<String> {
        match self.how {
            How::Configured => None,
            How::Remembered | How::Fresh => Some(format!(
                "🔑 API  {}\n   the bearer token for this box's own surfaces, kept in its store",
                self.token
            )),
            How::Ephemeral => Some(format!(
                "🔑 API  {}\n   this box keeps no store, so this token lasts until it restarts",
                self.token
            )),
        }
    }
}

/// How the box came by its token.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum How {
    /// A deployment provisioned it.
    Configured,
    /// The box issued it earlier and remembered it.
    #[default]
    Remembered,
    /// The box issued it just now and kept it.
    Fresh,
    /// The box issued it just now and has nowhere to keep it.
    Ephemeral,
}

/// A fresh token from the operating system's entropy, and nothing else.
///
/// Thirty-two bytes, hex. A credential derived from a site name, a serial number
/// or a clock is a credential somebody can compute, which is why `getrandom` is
/// the only source this workspace uses for one.
///
/// # Panics
/// When the operating system cannot produce entropy, which is a box that must
/// not go on to serve a surface it cannot protect.
fn generate() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system's entropy");
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Evidence for SR-035: a name that could forge an audit line is refused.
    #[tokio::test]
    async fn a_name_that_could_forge_an_audit_line_is_refused() {
        let access = LocalAccess::for_testing("reference-household", "s3cret");
        for forged in [
            "evil\nINFO s2{subject=cem:aggregator-nord}",
            "evil subject=cem:aggregator-nord",
            "",
            "-leading-dash",
            &"x".repeat(65),
        ] {
            assert!(
                matches!(
                    access.connect_manager(forged).await,
                    Err(ManagerError::Name(_))
                ),
                "{forged:?} must not name a manager"
            );
        }
        for fine in ["aggregator-nord", "Tibber_2", "cem.example"] {
            assert!(is_manager_name(fine), "{fine}");
        }
    }

    #[tokio::test]
    async fn a_token_opens_its_own_household_and_nothing_else() {
        let access = LocalAccess::for_testing("reference-household", "s3cret");
        let authority = access
            .authority(Some("Bearer s3cret"))
            .await
            .expect("the right token is accepted");
        assert!(authority.may_read("reference-household"));
        assert!(authority.may_write("reference-household"));
        assert!(
            !authority.may_read("somebody-elses-household"),
            "a box's own token is scoped to the box's own household"
        );
        assert!(
            !authority.may_read_the_fleet(),
            "and it is not a fleet credential"
        );
    }

    #[tokio::test]
    async fn every_way_of_not_presenting_a_token_is_refused() {
        let access = LocalAccess::for_testing("reference-household", "s3cret");
        for header in [
            None,
            Some(""),
            Some("s3cret"),
            Some("Bearer"),
            Some("Bearer "),
            Some("Bearer wrong"),
            Some("Basic s3cret"),
            // A prefix of the real token, which a comparison that stopped at the
            // first difference would take longer to refuse than a wrong one.
            Some("Bearer s3cre"),
        ] {
            assert!(
                access.authority(header).await.is_none(),
                "{header:?} should not open this box"
            );
        }
    }

    /// The token the box **prints** is the token the box **accepts**.
    ///
    /// Building the credential and the announcement from two calls to
    /// `generate()` compiles, passes every other test here, and produces a box
    /// nobody can log in to — it answers `401` to the number on its own screen.
    /// This is the assertion that the two come from one value.
    #[tokio::test]
    async fn the_token_announced_is_the_token_accepted() {
        let temporary = std::env::temp_dir().join(format!(
            "hems-access-{}.redb",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("a clock after 1970")
                .as_nanos()
        ));
        let store = Store::open(&temporary).expect("a fresh store");
        for expected in [How::Fresh, How::Remembered] {
            let (access, issued) =
                LocalAccess::resolve(&ApiSettings::default(), "reference-household", Some(&store))
                    .expect("a box can issue its own credential");
            assert_eq!(
                issued.how, expected,
                "the second resolve remembers the first"
            );
            assert!(
                access
                    .authority(Some(&format!("Bearer {}", issued.token)))
                    .await
                    .is_some(),
                "the box refused the token it printed"
            );
        }
        // …and the same for a box with nothing to remember it in.
        let (access, issued) =
            LocalAccess::resolve(&ApiSettings::default(), "reference-household", None)
                .expect("a box with no store still protects itself");
        assert_eq!(issued.how, How::Ephemeral);
        assert!(
            access
                .authority(Some(&format!("Bearer {}", issued.token)))
                .await
                .is_some(),
            "the box refused the token it printed"
        );
        drop(store);
        let _ = std::fs::remove_file(&temporary);
    }

    // Evidence for SR-069.
    #[tokio::test]
    async fn a_replaced_token_is_kept_and_the_old_one_is_dead() {
        let store = Arc::new(tokio::sync::Mutex::new(
            Store::in_memory().expect("a store"),
        ));
        let access =
            LocalAccess::for_testing("site-1", "the-old-one").remembering(Some(Arc::clone(&store)));
        let new = access.replace_token().await.expect("a new token");
        assert!(access.authority_of_token("the-old-one").await.is_none());
        assert!(access.authority_of_token(&new).await.is_some());
        assert_eq!(
            store.lock().await.api_token().expect("a read"),
            Some(new),
            "a restart has to answer to the new one, not the one on the lost sheet"
        );
    }

    #[tokio::test]
    async fn a_token_the_deployment_provisions_is_not_replaced_from_the_box() {
        let mut access = LocalAccess::for_testing("site-1", "provisioned");
        access.issued_here = false;
        assert!(matches!(
            access.replace_token().await,
            Err(ReplaceError::Configured)
        ));
        assert!(access.authority_of_token("provisioned").await.is_some());
    }

    #[test]
    fn a_generated_token_is_not_guessable_and_not_the_same_twice() {
        let first = generate();
        let second = generate();
        assert_eq!(first.len(), 64, "thirty-two bytes, hex");
        assert_ne!(first, second);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_configured_token_is_not_announced_and_an_issued_one_is() {
        // The operator who set it already has it; printing it again would put a
        // provisioned secret into a log that a generated one has to be in.
        assert_eq!(
            Issued {
                token: "x".into(),
                how: How::Configured
            }
            .announcement(),
            None
        );
        for how in [How::Remembered, How::Fresh, How::Ephemeral] {
            let line = Issued {
                token: "abc".into(),
                how,
            }
            .announcement()
            .expect("an issued token has to be shown to somebody");
            assert!(line.contains("abc"));
        }
    }
}
