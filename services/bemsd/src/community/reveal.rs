//! A coordinator may inspect only the readings revealed at this replay step.
use super::{Decimal, Reading};
use serde::Deserialize;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Reveal {
    None,
    A,
    B,
    Both,
}

impl Reveal {
    pub(super) fn project(self, values: &mut [Option<Decimal>]) {
        if matches!(self, Self::None | Self::B) {
            values[0] = None;
        }
        if matches!(self, Self::None | Self::A) {
            values[1] = None;
        }
    }

    pub(super) fn panel(self, reading: &Reading) -> String {
        let summary = reading.coordinator.as_ref().expect("coordinator replay");
        let bound = summary
            .minimum_residual_kwh
            .as_deref()
            .expect("selected replay");
        let supply = &summary.supply_kwh;
        let conclusion = match self {
            Self::None => {
                format!("Zwei Messwerte fehlen. Können alle {supply} kWh verteilt werden?")
            }
            Self::A | Self::B if bound != "0" => {
                format!("Mindestens {bound} kWh bleiben übrig. Ein Messwert fehlt.")
            }
            Self::A | Self::B => {
                format!("Alle {supply} kWh könnten verteilt werden. Ein Messwert fehlt.")
            }
            Self::Both => format!(
                "Abgeschlossen. {} kWh bleiben übrig.",
                summary.residual_kwh.as_deref().expect("complete fixture")
            ),
        };
        let steps: &[(&str, &str)] = match self {
            Self::None => &[("a", "Messwert A aufdecken"), ("b", "Messwert B aufdecken")],
            Self::A => &[
                ("both", "Messwert B aufdecken"),
                ("none", "Beide ausblenden"),
            ],
            Self::B => &[
                ("both", "Messwert A aufdecken"),
                ("none", "Beide ausblenden"),
            ],
            Self::Both => &[
                ("a", "Nur Messwert A zeigen"),
                ("b", "Nur Messwert B zeigen"),
            ],
        };
        let mut html = format!(
            r#"<section aria-labelledby="messfrage"><h2 id="messfrage" tabindex="-1">Welche Messung entscheidet?</h2><p>{conclusion}</p>"#
        );
        for (state, label) in steps {
            html.push_str(&format!(
                r##"<a href="?interval=2&amp;reveal={state}#messfrage">{label}</a> "##
            ));
        }
        html.push_str("</section>");
        html
    }
}

pub(super) fn minimum(values: &[Option<Decimal>], supply: Decimal) -> Decimal {
    let entitlement = supply / Decimal::TWO;
    values
        .iter()
        .flatten()
        .map(|demand| (entitlement - *demand).max(Decimal::ZERO))
        .sum()
}
