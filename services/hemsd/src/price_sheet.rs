//! Verify a household price sheet carried in a compact JWS.
use anyhow::{Context, Result, ensure};
use aws_lc_rs::{digest, signature};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use ems_core::prelude::Horizon;
use ems_tariff::sheet::PriceSheet;
use serde::Deserialize;

/// Maximum compact envelope for a 64 KiB document and its protected header.
pub const MAX_ENVELOPE: usize = 90 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    alg: String,
    kid: String,
    jwk: Key,
    typ: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Key {
    crv: String,
    kty: String,
    x: String,
}

/// Signature-checked content and identity for independent household comparison.
pub struct Verified {
    /// Parsed signed tariff.
    pub sheet: PriceSheet,
    /// Protected key identifier chosen by the signer.
    pub key_id: String,
    /// RFC 7638 SHA-256 thumbprint of the public JWK.
    pub fingerprint: String,
    /// SHA-256 of the exact compact envelope, for binding the approval.
    pub digest: String,
}

/// Verify the original signing input before parsing any price.
/// The caller establishes trust in the displayed public key independently.
///
/// # Errors
/// Returns an error for an invalid envelope, signature or sheet.
pub fn verify(compact: &[u8]) -> Result<Verified> {
    ensure!(
        compact.len() <= MAX_ENVELOPE,
        "price sheet envelope too large"
    );
    let compact = std::str::from_utf8(compact)?.trim();
    let (signed, sig) = compact.rsplit_once('.').context("missing JWS signature")?;
    let (protected, payload) = signed.split_once('.').context("missing JWS payload")?;
    ensure!(
        protected.len() <= 4096 && !payload.contains('.'),
        "invalid compact JWS"
    );
    let header: Header = serde_json::from_slice(&B64.decode(protected)?)?;
    ensure!(
        header.alg == "Ed25519" && header.jwk.kty == "OKP" && header.jwk.crv == "Ed25519",
        "expected an Ed25519 signing key"
    );
    ensure!(
        header.typ.as_deref().is_none_or(|t| t == "JOSE"),
        "unsupported JWS type"
    );
    ensure!(
        !header.kid.trim().is_empty()
            && header.kid.len() <= 128
            && !header.kid.chars().any(char::is_control),
        "invalid signer key ID"
    );
    let public = B64.decode(&header.jwk.x)?;
    let signature = B64.decode(sig)?;
    signature::UnparsedPublicKey::new(&signature::ED25519, public)
        .verify(signed.as_bytes(), &signature)
        .map_err(|_| anyhow::anyhow!("invalid price sheet signature"))?;
    let sheet = PriceSheet::parse(&B64.decode(payload)?)?;
    // RFC 8037 §2 uses precisely these lexically ordered public members.
    let canonical = format!(r#"{{"crv":"Ed25519","kty":"OKP","x":"{}"}}"#, header.jwk.x);
    let hash = |bytes: &[u8]| B64.encode(digest::digest(&digest::SHA256, bytes));
    Ok(Verified {
        sheet,
        key_id: header.kid,
        fingerprint: hash(canonical.as_bytes()),
        digest: hash(compact.as_bytes()),
    })
}

/// Ordered future tariff changes together with the last predecessor.
#[derive(Default)]
pub(crate) struct Schedule(Vec<(i64, Verified)>);

/// The complete market hours needed to price every quarter of a solve.
pub(crate) fn market_hours(horizon: Horizon) -> Horizon {
    let quarter = horizon.first.start().minute() / 15;
    Horizon {
        first: horizon.first.offset(-i64::from(quarter)),
        len: (usize::from(quarter) + horizon.len).next_multiple_of(4),
    }
}

impl Schedule {
    pub(crate) fn load(store: &crate::store::Store) -> Result<Self> {
        Ok(Self(
            store
                .price_sheets()?
                .into_iter()
                .map(|(start, compact)| Ok((start, verify(compact.as_bytes())?)))
                .collect::<Result<_>>()?,
        ))
    }

    pub(crate) fn previous(&self, slot: ems_core::prelude::Slot) -> Option<&Verified> {
        self.0
            .iter()
            .rev()
            .find(|(start, _)| *start <= slot.start().unix_timestamp())
            .map(|(_, item)| item)
    }

    pub(crate) fn applicable(&self, slot: ems_core::prelude::Slot) -> Option<&Verified> {
        self.previous(slot)
            .filter(|item| item.sheet.covers(slot.local_date()))
    }

    pub(crate) fn status(
        &self,
        slot: ems_core::prelude::Slot,
        published: Option<&ems_tariff::PriceStack>,
        fallback: Option<rust_decimal::Decimal>,
        can_import: bool,
    ) -> Status {
        let active = self.applicable(slot);
        let next = self.next(slot);
        let price = match active {
            Some(sold) => sold.import_price(slot, published, fallback),
            None => published
                .and_then(|stack| stack.at(slot))
                .filter(|p| p.contract.is_none())
                .cloned(),
        };
        Status {
            can_import,
            active: active.map(Terms::from),
            expired: self
                .previous(slot)
                .filter(|sold| !sold.sheet.covers(slot.local_date()))
                .map(Terms::from),
            next: next.map(|(_, item)| Terms::from(item)),
            next_at: next.map(|(start, _)| start),
            price_ct: price.as_ref().map(|p| p.import_ct.normalize().to_string()),
            estimated: price.is_some_and(|p| !p.price_known),
        }
    }

    /// Prices for the exact solve horizon and a publication covering its complete
    /// edge hours, so a mid-hour seller preview can still resolve an hourly mean.
    pub(crate) fn prices(
        &self,
        site: &ems_tariff::Tariff,
        horizon: ems_core::prelude::Horizon,
        market: &std::collections::BTreeMap<ems_core::prelude::Slot, rust_decimal::Decimal>,
        fallback: rust_decimal::Decimal,
    ) -> (ems_tariff::PriceStack, ems_tariff::PriceStack) {
        let prefix = usize::from(horizon.first.start().minute() / 15);
        let slots: Vec<_> = market_hours(horizon)
            .slots()
            .map(|slot| {
                self.applicable(slot)
                    .and_then(|sold| sold.sheet.site_price(slot, site, market, fallback))
                    .unwrap_or_else(|| {
                        ems_tariff::stack::price_at_market(site, slot, market.get(&slot).copied())
                    })
            })
            .collect();
        let plan = ems_tariff::PriceStack {
            slots: slots[prefix..prefix + horizon.len].to_vec(),
        };
        (plan, ems_tariff::PriceStack { slots })
    }

    /// Tie a preview to the schedule the household reviewed, including future changes.
    pub(crate) fn revision(&self) -> String {
        let mut hash = digest::Context::new(&digest::SHA256);
        for (start, item) in &self.0 {
            hash.update(&start.to_be_bytes());
            hash.update(item.digest.as_bytes());
        }
        B64.encode(hash.finish())
    }

    pub(crate) fn next(&self, slot: ems_core::prelude::Slot) -> Option<(i64, &Verified)> {
        self.0
            .iter()
            .find(|(start, _)| *start > slot.start().unix_timestamp())
            .map(|(start, item)| (*start, item))
    }

    pub(crate) fn can_schedule(&self, start: i64, now: i64) -> bool {
        self.0.iter().filter(|(time, _)| *time > now).count() < 8
            || self.0.iter().any(|(time, _)| *time == start)
    }
}

/// Identity and priced period the household can compare with its order.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, Deserialize, schemars::JsonSchema)]
pub struct Terms {
    /// Supplier's contract reference.
    pub contract: String,
    /// Protected signing key ID.
    pub key_id: String,
    /// RFC 7638 SHA-256 public-key thumbprint.
    pub fingerprint: String,
    /// First day stated on the signed sheet.
    pub valid_from: String,
    /// Last signed day, inclusive.
    pub valid_until: Option<String>,
    /// Calendar year for which charges were priced.
    pub priced_for_year: i32,
}

impl From<&Verified> for Terms {
    fn from(item: &Verified) -> Self {
        Self {
            contract: item.sheet.contract.clone(),
            key_id: item.key_id.clone(),
            fingerprint: item.fingerprint.clone(),
            valid_from: item.sheet.start.to_string(),
            valid_until: item.sheet.end.map(|d| d.to_string()),
            priced_for_year: item.sheet.year,
        }
    }
}

/// State read back after a household decision.
#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct Status {
    /// The box has durable storage for household approvals.
    pub can_import: bool,
    /// Contract in force now, if any.
    pub active: Option<Terms>,
    /// Last approved sheet when its period or priced year has ended.
    pub expired: Option<Terms>,
    /// Next scheduled contract, if any.
    pub next: Option<Terms>,
    /// Actual first quarter of the next contract, as Unix seconds.
    pub next_at: Option<i64>,
    /// Gross import price for the active terms in the current quarter.
    pub price_ct: Option<String>,
    /// A current price uses an estimated wholesale index.
    pub estimated: bool,
}

/// A signed file from the household, optionally paired with its exact reviewed facts.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct Import {
    /// Compact JWS carrying the original price sheet.
    pub signed: String,
    /// Required for activation.
    pub confirmation: Option<Review>,
}

/// Facts to display before committing; any change requires a fresh review.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize, schemars::JsonSchema)]
pub struct Review {
    /// Contract and signer to confirm.
    pub terms: Terms,
    /// SHA-256 of the submitted signed envelope.
    pub digest: String,
    /// Schedule as it stood at preview.
    pub revision: String,
    /// First eligible quarter as Unix seconds.
    pub effective_at: i64,
    /// Gross ct/kWh in the first eligible quarter, where the index is known.
    pub price_ct: Option<String>,
}

impl Verified {
    /// Resolve the signed import terms against the raw indices in the published plan.
    /// A preview requires a known index. Current status may use the site's fallback.
    pub(crate) fn import_price(
        &self,
        slot: ems_core::prelude::Slot,
        prices: Option<&ems_tariff::PriceStack>,
        fallback: Option<rust_decimal::Decimal>,
    ) -> Option<ems_tariff::SlotPrice> {
        use rust_decimal::Decimal;
        let market = prices
            .into_iter()
            .flat_map(|stack| &stack.slots)
            .filter_map(|price| price.market_ct.map(|value| (price.slot, value)))
            .collect();
        self.sheet
            .site_price(
                slot,
                &ems_tariff::Tariff::fixed(Decimal::ZERO, Decimal::ZERO),
                &market,
                fallback.unwrap_or_default(),
            )
            .filter(|price| price.price_known || fallback.is_some())
    }

    pub(crate) fn review(
        &self,
        schedule: &Schedule,
        now: ems_core::prelude::Slot,
        prices: Option<&ems_tariff::PriceStack>,
    ) -> Result<Review> {
        use ems_core::prelude::Slot;
        let january = time::Date::from_ordinal_date(self.sheet.year, 1)?;
        let first_day = january.max(self.sheet.start);
        let start = Slot::containing(ems_core::calendar::day_start_utc(first_day)).max(now.next());
        ensure!(
            self.sheet.covers(start.local_date()),
            "price sheet has expired"
        );
        ensure!(
            schedule.can_schedule(start.start().unix_timestamp(), now.start().unix_timestamp()),
            "too many future changes"
        );
        let price_ct = self
            .import_price(start, prices, None)
            .map(|price| price.import_ct.normalize().to_string());
        Ok(Review {
            terms: Terms::from(self),
            digest: self.digest.clone(),
            revision: schedule.revision(),
            effective_at: start.start().unix_timestamp(),
            price_ct,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
    use ems_core::prelude::Slot;
    use rust_decimal::{Decimal, dec};
    use time::macros::datetime;

    fn signed(doc: &serde_json::Value, seed: u8) -> String {
        let key = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
        let header = serde_json::json!({"alg":"Ed25519", "kid":"same printed ID", "jwk":{"kty":"OKP","crv":"Ed25519","x":B64.encode(key.public_key().as_ref())}});
        let data = format!(
            "{}.{}",
            B64.encode(header.to_string()),
            B64.encode(doc.to_string())
        );
        format!("{data}.{}", B64.encode(key.sign(data.as_bytes()).as_ref()))
    }

    #[test]
    fn gateway_review_and_plan_match_signed_seller_examples() {
        let fixtures = [
            (
                include_str!("../tests/fixtures/tariff/fixed.jws"),
                include_str!("../../../crates/ems-tariff/examples/preisblatt-fixed.json"),
            ),
            (
                include_str!("../tests/fixtures/tariff/dynamic.jws"),
                include_str!("../../../crates/ems-tariff/examples/preisblatt-dynamic.json"),
            ),
        ];
        let rows: Vec<(&str, Option<&str>, Option<&str>)> =
            serde_json::from_str(include_str!("../tests/fixtures/tariff/quotes.json")).unwrap();
        let site = ems_tariff::Tariff::fixed(dec!(10), dec!(7));
        let effective = Slot::containing(datetime!(2026-12-31 23:00 UTC));
        for (sheet_index, (compact, example)) in fixtures.into_iter().enumerate() {
            assert_eq!(compact.split('.').nth(1).unwrap(), B64.encode(example));
            let verified = verify(compact.as_bytes()).unwrap();
            let store = crate::store::Store::in_memory().unwrap();
            store
                .put_price_sheet(
                    effective.prev().start().unix_timestamp(),
                    effective.start().unix_timestamp(),
                    compact,
                )
                .unwrap();
            let schedule = Schedule::load(&store).unwrap();
            for &(instant, fixed, dynamic) in &rows {
                let at = time::OffsetDateTime::parse(
                    instant,
                    &time::format_description::well_known::Rfc3339,
                )
                .unwrap();
                let slot = Slot::starting_at(at).unwrap();
                let market = [(slot, dec!(-2))].into();
                let (plan, _) = schedule.prices(
                    &site,
                    ems_core::prelude::Horizon::new(at, 1),
                    &market,
                    Decimal::ZERO,
                );
                let price = plan.at(slot).unwrap();
                let expected = [fixed, dynamic][sheet_index];
                assert_eq!(
                    price.contract.as_deref(),
                    expected.map(|_| verified.sheet.contract.as_str())
                );
                if let Some(expected) = expected {
                    assert!(price.price_known);
                    let previous_plan = ems_tariff::PriceStack {
                        slots: vec![ems_tariff::stack::price_at_market(
                            &site,
                            slot,
                            Some(dec!(-2)),
                        )],
                    };
                    let status = schedule.status(slot, Some(&previous_plan), None, true);
                    assert_eq!(status.price_ct.as_deref(), Some(expected));
                    assert!(!status.estimated);
                    assert_eq!(status.active.unwrap().contract, verified.sheet.contract);
                    assert_eq!(price.import_ct.normalize().to_string(), expected);
                    let preview = verified
                        .review(&Schedule::default(), slot.prev(), Some(&plan))
                        .unwrap();
                    assert_eq!(preview.effective_at, at.unix_timestamp());
                    assert_eq!(preview.price_ct.as_deref(), Some(expected));
                }
            }
        }
    }

    #[test]
    fn mid_hour_status_and_review_keep_the_hourly_index_used_by_the_planner() {
        let mut doc: serde_json::Value = serde_json::from_str(include_str!(
            "../../../crates/ems-tariff/examples/preisblatt-dynamic.json"
        ))
        .unwrap();
        doc["energy"]["index"] = "day-ahead-hourly-mean".into();
        let compact = signed(&doc, 3);
        let first = Slot::containing(datetime!(2027-01-05 16:00 UTC));
        let at = first.offset(2);
        let store = crate::store::Store::in_memory().unwrap();
        store
            .put_price_sheet(
                first.prev().start().unix_timestamp(),
                first.start().unix_timestamp(),
                &compact,
            )
            .unwrap();
        let schedule = Schedule::load(&store).unwrap();
        let market = [
            (first, dec!(-2)),
            (first.next(), dec!(4)),
            (at, dec!(6)),
            (at.next(), dec!(8)),
        ]
        .into();
        let (plan, published) = schedule.prices(
            &ems_tariff::Tariff::fixed(dec!(10), dec!(7)),
            ems_core::prelude::Horizon::new(at.start(), 1),
            &market,
            dec!(12),
        );
        assert_eq!(plan.slots.len(), 1);
        assert_eq!(plan.at(at).unwrap().import_ct, dec!(37.16489));
        let status = schedule.status(at, Some(&published), Some(dec!(12)), true);
        assert_eq!(status.price_ct.as_deref(), Some("37.16489"));
        assert!(!status.estimated);
        let sold = schedule.applicable(at).unwrap();
        assert_eq!(
            sold.review(&schedule, at.prev(), Some(&published))
                .unwrap()
                .price_ct,
            status.price_ct
        );
        assert!(
            sold.review(&schedule, at.prev(), Some(&plan))
                .unwrap()
                .price_ct
                .is_none()
        );
        assert!(
            schedule
                .status(at, Some(&plan), Some(dec!(12)), true)
                .estimated
        );
    }

    #[test]
    fn schedule_reopens_at_berlin_midnight_and_factory_reset_discards_signer() {
        let before = Slot::containing(datetime!(2026-12-31 22:45 UTC));
        let today = before.prev();
        let mut doc: serde_json::Value = serde_json::from_str(include_str!(
            "../../../crates/ems-tariff/examples/preisblatt-fixed.json"
        ))
        .unwrap();
        let future = signed(&doc, 1);
        doc["contract"] = "old".into();
        doc["priced_for_year"] = 2026.into();
        doc["valid_from"] = "2026-01-01".into();
        doc["energy"]["ct_per_kwh"] = "4.80".into();
        let old = signed(&doc, 2);
        let path = std::env::temp_dir().join(format!("ems-signed-{}.redb", uuid::Uuid::new_v4()));
        {
            let store = crate::store::Store::open(&path).unwrap();
            store
                .put_price_sheet(
                    today.start().unix_timestamp(),
                    before.start().unix_timestamp(),
                    &old,
                )
                .unwrap();
            let schedule = Schedule::load(&store).unwrap();
            let verified = verify(future.as_bytes()).unwrap();
            let preview = verified.review(&schedule, today, None).unwrap();
            assert_eq!(preview.effective_at, before.next().start().unix_timestamp());
            assert_eq!(preview.price_ct.as_deref(), Some("33.71389"));
            let forged = signed(&doc, 1);
            let mut wrong = forged.rsplit_once('.').unwrap().0.to_owned();
            wrong.push('.');
            wrong.push_str(old.rsplit_once('.').unwrap().1);
            assert!(verify(wrong.as_bytes()).is_err());
            assert_ne!(
                verified.fingerprint,
                verify(old.as_bytes()).unwrap().fingerprint
            );
            store
                .put_price_sheet(
                    today.start().unix_timestamp(),
                    preview.effective_at,
                    &future,
                )
                .unwrap();
        }
        let mut store = crate::store::Store::open(&path).unwrap();
        let schedule = Schedule::load(&store).unwrap();
        let site = ems_tariff::Tariff::fixed(dec!(10), dec!(7));
        let (plan, _) = schedule.prices(
            &site,
            ems_core::prelude::Horizon::new(before.start(), 2),
            &std::collections::BTreeMap::new(),
            Decimal::ZERO,
        );
        let current = plan.at(before).unwrap();
        assert_eq!(current.contract.as_deref(), Some("old"));
        assert_eq!(current.import_ct, dec!(27.76389));
        let replacement = plan.at(before.next()).unwrap();
        assert_eq!(replacement.contract.as_deref(), Some("V-2026-004711"));
        assert_eq!(replacement.import_ct, dec!(33.71389));
        assert_eq!(replacement.annual_fixed_eur, dec!(-24.2998));
        store.factory_reset().unwrap();
        assert!(
            Schedule::load(&store)
                .unwrap()
                .applicable(before.next())
                .is_none()
        );
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}
