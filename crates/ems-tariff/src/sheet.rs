//! Offline inspection of a proposed seller price sheet. The caller supplies the
//! published index for the requested slot. Parsing never activates a contract.
use std::collections::BTreeMap;

use ems_core::prelude::Slot;
use ems_grid::modul3::{
    Modul3Calendar, Modul3Conformance, Modul3Context, Quarter, ZaehlzeitFenster,
    Zaehlzeitdefinition,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use time::{Date, Month, Time, macros::format_description};

use crate::{EnergyPrice, Levies, NetworkCharge, SlotPrice, Tariff, stack::price_at};

/// An invalid document, an out-of-period slot, or a missing index observation.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SheetError(String);

fn invalid(field: &str) -> SheetError {
    SheetError(format!("invalid or missing {field}"))
}

fn amount(text: &str, name: &str) -> Result<Decimal, SheetError> {
    Decimal::from_str_exact(text)
        .ok()
        .filter(|v| v.abs() <= Decimal::from(1_000_000) && v.scale() <= 10)
        .ok_or_else(|| invalid(name))
}

#[derive(Deserialize)]
struct Document {
    preisblatt: u8,
    contract: String,
    valid_from: String,
    valid_until: Option<String>,
    priced_for_year: i32,
    energy: Energy,
    network: Network,
    components_ct_per_kwh: Components,
    vat_rate: String,
    standing_eur_per_year: Option<Standing>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Energy {
    Fixed {
        ct_per_kwh: String,
    },
    Dynamic {
        index: String,
        bidding_zone: String,
        markup_ct_per_kwh: String,
    },
}

#[derive(Deserialize)]
struct Network {
    modul: u8,
    arbeitspreis_ct_per_kwh: Option<String>,
    reduction_eur_per_year: Option<String>,
    remaining_share: Option<String>,
    metering_eur_per_year: Option<String>,
    ht_ct_per_kwh: Option<String>,
    st_ct_per_kwh: Option<String>,
    nt_ct_per_kwh: Option<String>,
    hochtarif: Option<Vec<[String; 2]>>,
    niedertarif: Option<Vec<[String; 2]>>,
    billed_quarters: Option<Vec<String>>,
    source: Option<String>,
}

#[derive(Deserialize)]
struct Components {
    konzessionsabgabe: String,
    stromsteuer: Option<String>,
    paragraph_19_strom_nev: Option<String>,
    kwkg: Option<String>,
    offshore: Option<String>,
}

#[derive(Deserialize)]
struct Standing {
    grundpreis: String,
    messstellenbetrieb: String,
}

fn required(value: Option<&str>, name: &str) -> Result<Decimal, SheetError> {
    amount(value.ok_or_else(|| invalid(name))?, name)
}

fn minute(s: &str, end: bool) -> Result<u16, SheetError> {
    if end && s == "24:00" {
        return Ok(1440);
    }
    let t = Time::parse(s, &format_description!("[hour]:[minute]"))
        .map_err(|_| invalid("window time"))?;
    Ok(u16::from(t.hour()) * 60 + u16::from(t.minute()))
}

impl Network {
    fn charge(&self, id: &str, year: i32) -> Result<NetworkCharge, SheetError> {
        let work = || {
            required(
                self.arbeitspreis_ct_per_kwh.as_deref(),
                "arbeitspreis_ct_per_kwh",
            )
        };
        let reduction = || {
            required(
                self.reduction_eur_per_year.as_deref(),
                "reduction_eur_per_year",
            )
        };
        Ok(match self.modul {
            0 => NetworkCharge::None {
                arbeitspreis: work()?,
            },
            1 => NetworkCharge::Modul1 {
                arbeitspreis: work()?,
                reduction_eur_per_year: reduction()?,
            },
            2 => {
                let share = required(self.remaining_share.as_deref(), "remaining_share")?;
                if !(Decimal::ZERO..=Decimal::ONE).contains(&share) {
                    return Err(invalid("remaining_share"));
                }
                NetworkCharge::Modul2 {
                    arbeitspreis: work()?,
                    remaining_share: share,
                    metering_eur_per_year: required(
                        self.metering_eur_per_year.as_deref(),
                        "metering_eur_per_year",
                    )?,
                }
            }
            3 => NetworkCharge::Modul3 {
                calendar: self.calendar(id, year)?,
                ht: required(self.ht_ct_per_kwh.as_deref(), "ht_ct_per_kwh")?,
                st: required(self.st_ct_per_kwh.as_deref(), "st_ct_per_kwh")?,
                nt: required(self.nt_ct_per_kwh.as_deref(), "nt_ct_per_kwh")?,
                reduction_eur_per_year: reduction()?,
            },
            _ => return Err(invalid("network.modul")),
        })
    }

    fn calendar(&self, id: &str, year: i32) -> Result<Modul3Calendar, SheetError> {
        let source = self
            .source
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| invalid("network.source"))?;
        let mut quarters = Vec::new();
        let mut mask = 0;
        for q in self
            .billed_quarters
            .as_ref()
            .ok_or_else(|| invalid("billed_quarters"))?
        {
            let quarter = match q.as_str() {
                "Q1" => Quarter::Q1,
                "Q2" => Quarter::Q2,
                "Q3" => Quarter::Q3,
                "Q4" => Quarter::Q4,
                _ => return Err(invalid("billed_quarters")),
            };
            if quarters.contains(&quarter) {
                return Err(invalid("duplicate billed quarter"));
            }
            mask |= quarter.month_mask();
            quarters.push(quarter);
        }
        if quarters.len() < 2 {
            return Err(invalid("billed_quarters"));
        }
        let mut windows = Vec::new();
        let mut occupied = [false; 1440];
        for (register, bands) in [("HT", &self.hochtarif), ("NT", &self.niedertarif)] {
            for [start, end] in bands.as_ref().ok_or_else(|| invalid(register))? {
                let (start, end) = (minute(start, false)?, minute(end, true)?);
                if start == end || start % 15 != 0 || end % 15 != 0 {
                    return Err(invalid("quarter-hour window"));
                }
                for m in start..if end > start { end } else { end + 1440 } {
                    if std::mem::replace(&mut occupied[usize::from(m % 1440)], true) {
                        return Err(invalid("overlapping windows"));
                    }
                }
                for w in ZaehlzeitFenster::spanning(register, start, end)
                    .map_err(|_| invalid(register))?
                {
                    windows.push(w.in_months(mask).map_err(|_| invalid("billed_quarters"))?);
                }
            }
        }
        let date = |month, day| {
            Date::from_calendar_date(year, month, day).map_err(|_| invalid("priced_for_year"))
        };
        let definition =
            Zaehlzeitdefinition::new(id, date(Month::January, 1)?, windows, Some("ST".into()))
                .until(date(Month::December, 31)?);
        let calendar = Modul3Calendar {
            definition,
            billed_quarters: quarters,
            source: Some(source.into()),
        };
        let context = Modul3Context::new()
            .modul_1(true)
            .intelligentes_messsystem(true)
            .registrierende_leistungsmessung(false);
        if calendar.assess(&context).0 != Modul3Conformance::Conforms {
            return Err(invalid("Modul 3 calendar"));
        }
        Ok(calendar)
    }
}

/// One validated document and the national figures used for omitted components.
pub struct PriceSheet {
    /// Supplier's opaque contract reference.
    pub contract: String,
    /// Dynamic index the caller must provide, if any.
    pub index: Option<String>,
    /// National figures substituted from the sheet's priced year.
    pub published_components: Vec<&'static str>,
    /// First calendar day of the contract.
    pub start: Date,
    /// Last calendar day, inclusive.
    pub end: Option<Date>,
    /// Year covered by this signed set of levies.
    pub year: i32,
    tariff: Tariff,
}

impl PriceSheet {
    /// Read a local JSON payload, independently of signature verification.
    pub fn parse(bytes: &[u8]) -> Result<Self, SheetError> {
        if bytes.len() > 65536 {
            return Err(invalid("sheet size (maximum 64 KiB)"));
        }
        let doc: Document = serde_json::from_slice(bytes).map_err(|e| SheetError(e.to_string()))?;
        if doc.preisblatt != 1 {
            return Err(invalid("preisblatt version"));
        }
        if doc.contract.trim().is_empty()
            || doc.contract.len() > 256
            || doc.contract.chars().any(char::is_control)
        {
            return Err(invalid("contract"));
        }
        let date = |s: &str, name| {
            Date::parse(s, &format_description!("[year]-[month]-[day]")).map_err(|_| invalid(name))
        };
        let start = date(&doc.valid_from, "valid_from")?;
        let end = doc
            .valid_until
            .as_deref()
            .map(|s| date(s, "valid_until"))
            .transpose()?;
        if end.is_some_and(|e| e < start) {
            return Err(invalid("valid_until"));
        }
        if !(1..=9999).contains(&doc.priced_for_year)
            || doc.priced_for_year < start.year()
            || end.is_some_and(|e| doc.priced_for_year > e.year())
        {
            return Err(invalid("priced_for_year"));
        }
        let mut tariff = Tariff::fixed(Decimal::ZERO, Decimal::ZERO);
        let index = match doc.energy {
            Energy::Fixed { ct_per_kwh } => {
                tariff.energy = EnergyPrice::Fixed {
                    ct_per_kwh: amount(&ct_per_kwh, "ct_per_kwh")?,
                };
                None
            }
            Energy::Dynamic {
                index,
                bidding_zone,
                markup_ct_per_kwh,
            } => {
                if bidding_zone != "DE-LU" {
                    return Err(invalid("bidding_zone"));
                }
                if !matches!(
                    index.as_str(),
                    "day-ahead-quarter-hour" | "day-ahead-hourly-mean"
                ) {
                    return Err(invalid("energy.index"));
                }
                tariff.energy = EnergyPrice::Dynamic {
                    spot: BTreeMap::new(),
                    markup_ct_per_kwh: amount(&markup_ct_per_kwh, "markup_ct_per_kwh")?,
                    fallback_ct_per_kwh: Decimal::ZERO,
                };
                Some(index)
            }
        };
        tariff.network = doc.network.charge(&doc.contract, doc.priced_for_year)?;
        let published = Levies::household(doc.priced_for_year);
        let mut published_components = Vec::new();
        let mut component = |v: Option<String>, name, value: fn(Levies) -> Decimal| {
            if let Some(s) = v {
                amount(&s, name)
            } else {
                published_components.push(name);
                published.map(value).ok_or_else(|| invalid(name))
            }
        };
        let c = doc.components_ct_per_kwh;
        tariff.levies = Levies {
            konzessionsabgabe: amount(&c.konzessionsabgabe, "konzessionsabgabe")?,
            stromsteuer: component(c.stromsteuer, "stromsteuer", |l| l.stromsteuer)?,
            para19: component(c.paragraph_19_strom_nev, "paragraph_19_strom_nev", |l| {
                l.para19
            })?,
            kwkg: component(c.kwkg, "kwkg", |l| l.kwkg)?,
            offshore: component(c.offshore, "offshore", |l| l.offshore)?,
            vat_rate: amount(&doc.vat_rate, "vat_rate")?,
        };
        if !(Decimal::ZERO..=Decimal::ONE).contains(&tariff.levies.vat_rate) {
            return Err(invalid("vat_rate"));
        }
        tariff.levies_by_year.clear();
        if let Some(s) = doc.standing_eur_per_year {
            tariff.standing_charge_eur_per_year = amount(&s.grundpreis, "grundpreis")?
                + amount(&s.messstellenbetrieb, "messstellenbetrieb")?;
        }
        Ok(Self {
            contract: doc.contract,
            index,
            published_components,
            start,
            end,
            year: doc.priced_for_year,
            tariff,
        })
    }

    /// Whether this contract supplies prices for this Berlin calendar day.
    #[must_use]
    pub fn covers(&self, date: Date) -> bool {
        date >= self.start && self.end.is_none_or(|end| date <= end) && date.year() == self.year
    }

    /// Resolve the import terms with the site's separately contracted export and
    /// community terms. An hourly-mean index needs all four observations of the
    /// UTC delivery hour, including on a clock-change day.
    #[must_use]
    pub fn site_price(
        &self,
        slot: Slot,
        site: &Tariff,
        market: &BTreeMap<Slot, Decimal>,
        fallback: Decimal,
    ) -> Option<SlotPrice> {
        if !self.covers(slot.local_date()) {
            return None;
        }
        let market_at = market.get(&slot).copied();
        let mut terms = self.tariff.clone();
        if let EnergyPrice::Dynamic {
            spot,
            fallback_ct_per_kwh,
            ..
        } = &mut terms.energy
        {
            let index = if self.index.as_deref() == Some("day-ahead-hourly-mean") {
                let first = slot.offset(-i64::from(slot.start().minute() / 15));
                (0..4)
                    .try_fold(Decimal::ZERO, |sum, i| {
                        sum.checked_add(*market.get(&first.offset(i))?)
                    })
                    .and_then(|v| v.checked_div(Decimal::from(4)))
            } else {
                market_at
            };
            if let Some(value) = index {
                spot.insert(slot, value);
            }
            *fallback_ct_per_kwh = fallback;
        }
        terms.feed_in = site.feed_in.clone();
        terms.sharing = site.sharing;
        let mut price = crate::stack::price_at_market(&terms, slot, market_at);
        price.co2_g_per_kwh = site.carbon_g_per_kwh.get(&slot).copied();
        price.annual_fixed_eur = terms.levies.gross(price.annual_fixed_eur);
        price.contract = Some(self.contract.clone());
        Some(price)
    }

    /// Quote one quarter at the supplied contract index (net ct/kWh).
    /// Outside the contract or priced year the caller must select another sheet.
    pub fn quote(&self, slot: Slot, index_ct: Option<Decimal>) -> Result<SlotPrice, SheetError> {
        let date = slot.local_date();
        if date < self.start || self.end.is_some_and(|end| date > end) {
            return Err(invalid("contract validity for this slot"));
        }
        if date.year() != self.year {
            return Err(invalid("priced_for_year for this slot"));
        }
        let mut tariff = self.tariff.clone();
        match (&mut tariff.energy, index_ct) {
            (EnergyPrice::Dynamic { spot, .. }, Some(index))
                if index.abs() <= Decimal::from(1_000_000) && index.scale() <= 10 =>
            {
                spot.insert(slot, index);
            }
            (EnergyPrice::Fixed { .. }, None) => (),
            _ => return Err(invalid("index price for this energy contract")),
        }
        Ok(price_at(&tariff, slot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::macros::datetime;

    const FIXED: &[u8] = include_bytes!("../examples/preisblatt-fixed.json");
    const DYNAMIC: &[u8] = include_bytes!("../examples/preisblatt-dynamic.json");

    #[test]
    fn fixed_price_begins_at_local_midnight() {
        let sheet = PriceSheet::parse(FIXED).unwrap();
        let first = Slot::containing(datetime!(2026-12-31 23:00 UTC));
        assert!(sheet.quote(first.prev(), None).is_err());
        assert_eq!(sheet.quote(first, None).unwrap().import_ct, dec!(33.71389));
        assert!(sheet.quote(first, Some(dec!(3))).is_err());
        assert!(
            sheet
                .quote(Slot::containing(datetime!(2027-12-31 23:00 UTC)), None)
                .is_err()
        );
    }

    #[test]
    fn dynamic_quote_uses_the_supplied_index_and_local_windows() {
        let sheet = PriceSheet::parse(DYNAMIC).unwrap();
        let high = Slot::containing(datetime!(2027-01-05 16:00 UTC));
        let quote = |slot| sheet.quote(slot, Some(dec!(-2))).unwrap().import_ct;
        assert_eq!(quote(high.prev()), dec!(20.50489));
        assert_eq!(quote(high), dec!(30.02489));
        assert_eq!(quote(high.offset(20)), dec!(13.36489));
        assert_eq!(quote(high.offset(51)), dec!(13.36489));
        assert_eq!(quote(high.offset(52)), dec!(20.50489));
        assert!(sheet.quote(high, None).is_err());
    }

    #[test]
    fn hourly_import_and_quarter_hour_export_use_their_own_index() {
        let mut doc: serde_json::Value = serde_json::from_slice(DYNAMIC).unwrap();
        doc["energy"]["index"] = "day-ahead-hourly-mean".into();
        let sheet = PriceSheet::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
        let hour = Slot::containing(datetime!(2027-01-05 16:00 UTC));
        let mut spot = BTreeMap::from([
            (hour, dec!(-2)),
            (hour.next(), dec!(4)),
            (hour.offset(2), dec!(6)),
            (hour.offset(3), dec!(8)),
        ]);
        let site = Tariff::fixed(dec!(12), dec!(7));
        let quote = sheet.site_price(hour, &site, &spot, dec!(10)).unwrap();
        assert!(quote.price_known);
        assert_eq!(quote.energy_ct, dec!(5.9));
        assert_eq!(quote.market_ct, Some(dec!(-2)));
        spot.remove(&hour.offset(3));
        let gap = sheet.site_price(hour, &site, &spot, dec!(10)).unwrap();
        assert!(!gap.price_known);
        assert_eq!(gap.energy_ct, dec!(11.9));
    }

    #[test]
    fn bounded_documents_and_exact_money_are_required() {
        assert!(
            PriceSheet::parse(&vec![b' '; 65_537])
                .err()
                .unwrap()
                .to_string()
                .contains("size")
        );
        let mut doc: serde_json::Value = serde_json::from_slice(FIXED).unwrap();
        doc["energy"]["ct_per_kwh"] = serde_json::json!("1.2345678901234567890123456789012345");
        assert!(
            PriceSheet::parse(&serde_json::to_vec(&doc).unwrap())
                .err()
                .unwrap()
                .to_string()
                .contains("ct_per_kwh")
        );
        doc["priced_for_year"] = 2020.into();
        assert!(
            PriceSheet::parse(&serde_json::to_vec(&doc).unwrap())
                .err()
                .unwrap()
                .to_string()
                .contains("priced_for_year")
        );
    }

    #[test]
    fn components_name_the_source_and_ambiguous_windows_are_rejected() {
        let mut doc: serde_json::Value = serde_json::from_slice(DYNAMIC).unwrap();
        doc["components_ct_per_kwh"]
            .as_object_mut()
            .unwrap()
            .remove("kwkg");
        let parse = |doc: &serde_json::Value| PriceSheet::parse(&serde_json::to_vec(doc).unwrap());
        assert_eq!(parse(&doc).unwrap().published_components, vec!["kwkg"]);
        doc["network"]["hochtarif"] = serde_json::json!([["05:45", "08:00"]]);
        assert!(
            parse(&doc)
                .err()
                .unwrap()
                .to_string()
                .contains("overlapping windows")
        );
        doc["preisblatt"] = 2.into();
        assert!(parse(&doc).err().unwrap().to_string().contains("version"));
    }
}
