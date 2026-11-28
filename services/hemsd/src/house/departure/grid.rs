//! Directional register evidence for the three OCPP quarters.
use anyhow::{Context, Result, ensure};
use ems_core::prelude::Measurement;
use time::Duration;

#[derive(Debug, serde::Serialize)]
pub(super) struct Balance {
    pub import: u32,
    pub export: u32,
    pub interruption: u32,
}

impl Balance {
    pub(super) fn from_samples(samples: [&Measurement; 4]) -> Result<Self> {
        use rust_decimal::prelude::ToPrimitive;
        let counters = |sample: &Measurement| -> Result<(u32, u32)> {
            let wh = |energy: Option<ems_core::prelude::Energy>| {
                energy
                    .map(ems_core::prelude::Energy::get)
                    .filter(|wh| wh.fract() == 0.0)
                    .and_then(|wh| wh.to_u32())
                    .context("missing whole-Wh connection register")
            };
            Ok((wh(sample.energy_in)?, wh(sample.energy_out)?))
        };
        let [open, pause, resume, close] = samples.map(counters);
        let points = [open?, pause?, resume?, close?];
        for (pair, registers) in samples.windows(2).zip(points.windows(2)) {
            ensure!(
                pair[1].at - pair[0].at == Duration::minutes(15),
                "connection readings must span consecutive quarters"
            );
            let (before, after) = (registers[0], registers[1]);
            ensure!(
                after.0 >= before.0 && after.1 >= before.1,
                "connection register moved backwards"
            );
        }
        let [open, pause, resume, close] = points;
        Ok(Self {
            import: close.0 - open.0,
            export: close.1 - open.1,
            interruption: resume.0 - pause.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ems_core::prelude::{Energy, Power};
    use time::OffsetDateTime;

    #[test]
    fn only_complete_consecutive_directional_registers_close_the_window() {
        let samples = [0_i32, 1, 2, 3].map(|i| {
            let mut sample = Measurement::power(
                OffsetDateTime::UNIX_EPOCH + Duration::minutes(i64::from(i) * 15),
                Power::ZERO,
            );
            sample.energy_in = Some(Energy::new(100.0 + 5.0 * f64::from(i)));
            sample.energy_out = Some(Energy::new(50.0 + f64::from(i)));
            sample
        });
        let parse = |s: &[Measurement; 4]| Balance::from_samples(s.each_ref());
        let balance = parse(&samples).unwrap();
        assert_eq!(
            (balance.import, balance.export, balance.interruption),
            (15, 3, 5)
        );

        let mut broken = samples;
        broken[1].energy_in = None;
        assert!(parse(&broken).is_err());
        broken = samples;
        broken[2].energy_out = Some(Energy::ZERO);
        assert!(parse(&broken).is_err());
        broken = samples;
        broken[1].at = broken[0].at;
        assert!(parse(&broken).is_err());
    }
}
