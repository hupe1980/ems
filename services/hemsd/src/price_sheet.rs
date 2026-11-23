//! Verify a household price sheet carried in a compact JWS.
use anyhow::{Context, Result, ensure};
use aws_lc_rs::{digest, signature};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64};
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

    pub(crate) fn prices(
        &self,
        site: &ems_tariff::Tariff,
        horizon: ems_core::prelude::Horizon,
        market: &std::collections::BTreeMap<ems_core::prelude::Slot, rust_decimal::Decimal>,
        fallback: rust_decimal::Decimal,
    ) -> ems_tariff::PriceStack {
        let slots = horizon
            .slots()
            .map(|slot| {
                self.applicable(slot)
                    .and_then(|sold| sold.sheet.site_price(slot, site, market, fallback))
                    .unwrap_or_else(|| {
                        ems_tariff::stack::price_at_market(site, slot, market.get(&slot).copied())
                    })
            })
            .collect();
        ems_tariff::PriceStack { slots }
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
    /// Gross import price in the current published plan.
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
    pub(crate) fn review(
        &self,
        schedule: &Schedule,
        now: ems_core::prelude::Slot,
        prices: Option<&ems_tariff::PriceStack>,
    ) -> Result<Review> {
        use ems_core::prelude::Slot;
        use rust_decimal::Decimal;
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
        let market = prices
            .into_iter()
            .flat_map(|p| &p.slots)
            .filter_map(|p| p.market_ct.map(|v| (p.slot, v)))
            .collect();
        let price_ct = self
            .sheet
            .site_price(
                start,
                &ems_tariff::Tariff::fixed(Decimal::ZERO, Decimal::ZERO),
                &market,
                Decimal::ZERO,
            )
            .filter(|p| p.price_known)
            .map(|p| p.import_ct.normalize().to_string());
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
        let plan = schedule.prices(
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
