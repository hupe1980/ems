//! Levies and taxes — the part of the bill that changes once a year.
//!
//! Everything here is in cents per kilowatt-hour and applies to imported
//! energy. Three of the figures are set nationally and published every October
//! for the year that follows; they are kept here as a dated table, held to the
//! publication by a test. The Konzessionsabgabe depends on the municipality and
//! the value added tax on the law, so both are defaults a supplier's price
//! sheet overrides — the only place any of this is authoritative for one
//! customer.

use std::collections::BTreeMap;

use rust_decimal::Decimal;

/// The levies and taxes on a kilowatt-hour drawn from the grid, ct/kWh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Levies {
    /// Stromsteuer (§ 3 StromStG).
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub stromsteuer: Decimal,
    /// KWKG-Umlage.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub kwkg: Decimal,
    /// § 19 StromNEV-Umlage — since 2025 the Aufschlag für besondere
    /// Netznutzung, at the rate for consumption up to 1 GWh a year.
    #[cfg_attr(
        feature = "serde",
        serde(rename = "paragraph_19_strom_nev", with = "rust_decimal::serde::str")
    )]
    pub para19: Decimal,
    /// Offshore-Netzumlage.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub offshore: Decimal,
    /// Konzessionsabgabe — depends on the municipality and the tariff type.
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub konzessionsabgabe: Decimal,
    /// Value added tax, as a fraction (0.19 for the ordinary rate).
    #[cfg_attr(feature = "serde", serde(with = "rust_decimal::serde::str"))]
    pub vat_rate: Decimal,
}

/// One year's nationally set figures, ct/kWh, and where they were published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Published {
    /// The calendar year the figures apply to.
    pub year: i32,
    /// Stromsteuer (§ 3 StromStG).
    pub stromsteuer: Decimal,
    /// KWKG-Umlage.
    pub kwkg: Decimal,
    /// Aufschlag für besondere Netznutzung (§ 19 StromNEV), up to 1 GWh a year.
    pub para19: Decimal,
    /// Offshore-Netzumlage.
    pub offshore: Decimal,
    /// The publication the figures were transcribed from.
    pub source: &'static str,
}

/// Thousandths of a cent, so the table reads as the publication does.
const fn ct(thousandths: u32) -> Decimal {
    Decimal::from_parts(thousandths, 0, 0, false, 3)
}

/// The Stromsteuer's ordinary rate, 20,50 €/MWh (§ 3 StromStG).
const STROMSTEUER: Decimal = Decimal::from_parts(205, 0, 0, false, 2);

/// What the transmission system operators published, oldest first. A year is
/// added when it is published, never before: a figure nobody has published is
/// not a figure.
pub const PUBLISHED: [Published; 3] = [
    Published {
        year: 2025,
        stromsteuer: STROMSTEUER,
        kwkg: ct(277),
        para19: ct(1_558),
        offshore: ct(816),
        source: "netztransparenz.de, Umlagen und Aufschläge 2025 (Veröffentlichung Oktober 2024)",
    },
    Published {
        year: 2026,
        stromsteuer: STROMSTEUER,
        kwkg: ct(446),
        para19: ct(1_559),
        offshore: ct(941),
        source: "netztransparenz.de, Umlagen und Aufschläge 2026 (Veröffentlichung Oktober 2025)",
    },
    Published {
        year: 2027,
        stromsteuer: STROMSTEUER,
        kwkg: ct(451),
        para19: ct(2_027),
        offshore: ct(1_143),
        source: "netztransparenz.de, Umlagen und Aufschläge 2027 (Veröffentlichung Oktober 2026)",
    },
];

/// The Konzessionsabgabe a household is charged where nothing says otherwise:
/// the ceiling for a municipality of 25 001 to 100 000 inhabitants, outside a
/// Schwachlast tariff (§ 2 Abs. 2 Nr. 1 b KAV). The other bands are 1,32, 1,99
/// and 2,39 ct/kWh, which is why the price sheet decides.
pub const KONZESSIONSABGABE_DEFAULT: Decimal = Decimal::from_parts(159, 0, 0, false, 2);

/// The ordinary rate of value added tax (§ 12 Abs. 1 UStG).
pub const VAT_DEFAULT: Decimal = Decimal::from_parts(19, 0, 0, false, 2);

impl From<Published> for Levies {
    fn from(p: Published) -> Self {
        Self {
            stromsteuer: p.stromsteuer,
            kwkg: p.kwkg,
            para19: p.para19,
            offshore: p.offshore,
            konzessionsabgabe: KONZESSIONSABGABE_DEFAULT,
            vat_rate: VAT_DEFAULT,
        }
    }
}

impl Levies {
    /// The sum of the levies before value added tax, ct/kWh.
    #[must_use]
    pub fn sum_net(&self) -> Decimal {
        self.stromsteuer + self.kwkg + self.para19 + self.offshore + self.konzessionsabgabe
    }

    /// Apply value added tax to a net price in ct/kWh.
    #[must_use]
    pub fn gross(&self, net: Decimal) -> Decimal {
        net * (Decimal::ONE + self.vat_rate)
    }

    /// A household's levies in `year`, where that year's figures have been
    /// published.
    #[must_use]
    pub fn household(year: i32) -> Option<Self> {
        PUBLISHED
            .iter()
            .find(|p| p.year == year)
            .map(|p| Self::from(*p))
    }

    /// The figures to use in `year`, and the year they were published for:
    /// that year's, else the last published before it, else the first there
    /// are. A caller that gets a different year back is running on figures
    /// that are not that year's, and should say so.
    #[must_use]
    pub fn latest_published(year: i32) -> (i32, Self) {
        let p = PUBLISHED
            .iter()
            .rev()
            .find(|p| p.year <= year)
            .unwrap_or(&PUBLISHED[0]);
        (p.year, Self::from(*p))
    }

    /// Every published year, for a tariff that prices each quarter hour on its
    /// own year's figures.
    #[must_use]
    pub fn published() -> BTreeMap<i32, Self> {
        PUBLISHED.iter().map(|p| (p.year, Self::from(*p))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    #[test]
    fn the_table_holds_the_published_figures() {
        // Transcribed here a second time, from the publications, so a slip in
        // the table has to be made twice to go unnoticed.
        let published = [
            (2025, dec!(0.277), dec!(1.558), dec!(0.816)),
            (2026, dec!(0.446), dec!(1.559), dec!(0.941)),
            (2027, dec!(0.451), dec!(2.027), dec!(1.143)),
        ];
        for (year, kwkg, para19, offshore) in published {
            let l = Levies::household(year).unwrap();
            assert_eq!(
                (l.kwkg, l.para19, l.offshore),
                (kwkg, para19, offshore),
                "{year}"
            );
            assert_eq!(l.stromsteuer, dec!(2.05), "{year}");
        }
        assert_eq!(PUBLISHED.len(), published.len());
        assert!(
            PUBLISHED
                .iter()
                .all(|p| p.source.contains(&p.year.to_string()))
        );
        assert!(PUBLISHED.windows(2).all(|w| w[0].year < w[1].year));
    }

    #[test]
    fn an_unpublished_year_is_none_and_falls_back_by_name() {
        assert!(Levies::household(2028).is_none());
        assert_eq!(Levies::latest_published(2028).0, 2027);
        assert_eq!(Levies::latest_published(2026).0, 2026);
        assert_eq!(Levies::latest_published(2019).0, 2025);
    }

    #[test]
    fn the_levies_add_up_and_take_vat() {
        let l = Levies::household(2026).unwrap();
        // 2,05 + 0,446 + 1,559 + 0,941 + 1,59.
        assert_eq!(l.sum_net(), dec!(6.586));
        assert_eq!(l.gross(l.sum_net()), dec!(7.83734));
    }

    #[test]
    fn a_customer_can_override_every_component() {
        let l = Levies {
            konzessionsabgabe: Decimal::ZERO,
            ..Levies::household(2026).unwrap()
        };
        assert_eq!(l.sum_net(), dec!(4.996));
    }
}
