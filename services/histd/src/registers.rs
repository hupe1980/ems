//! MeterStore's PostgreSQL hot tier holds one immutable assertion per site, quarter and revision.
use crate::{StoreError, config::RegisterMeter};
use datafusion_common::ScalarValue;
use ems_grid::mispel::QuarterHour;
use metering::{
    QualityFlag, Resolution,
    series::interval::{MeterInterval, Sparte},
};
use meterstore::{
    Database, HotStore, MeasurementSeries, MeasurementSource, PostgresHot, ScopedVersion,
    TableConfig, Version, VersionScope,
    arrow::datatypes::{DataType, Field},
    encode::StoredSeries,
};
use std::collections::{BTreeMap, BTreeSet};
use time::{Duration, OffsetDateTime};

const TABLE: &str = "histd_registers";
const FIELDS: [&str; 7] = [
    "feed",
    "consumed",
    "generated",
    "storage_in",
    "storage_out",
    "aw",
    "spot",
];
pub(crate) const WINDOW: &str = r#"SELECT DISTINCT ON ("from") "from", trim_scale(value),
    feed::numeric, consumed::numeric, generated::numeric, storage_in::numeric, storage_out::numeric,
    aw::numeric, spot::numeric FROM histd_registers
    WHERE site_id = $1 AND "from" >= $2 AND "from" < $3 AND recorded_at <= $4
    ORDER BY "from", version DESC"#;

#[derive(Debug)]
pub(crate) struct Registers {
    hot: PostgresHot,
    config: meterstore::ValidatedTableConfig,
    pub meters: BTreeMap<String, RegisterMeter>,
}

impl Registers {
    pub async fn open(
        settings: &ems_service::DbSettings,
        meters: BTreeMap<String, RegisterMeter>,
    ) -> Result<Self, StoreError> {
        let mut url =
            url::Url::parse(&settings.url().map_err(|_| {
                StoreError::InvalidAssertion("cannot resolve historian database URL")
            })?)
            .map_err(|_| StoreError::InvalidAssertion("MeterStore needs a PostgreSQL URL"))?;
        // Match the serving pool's explicit TLS policy on every MeterStore connection.
        let retained: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| key != "sslmode")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        url.query_pairs_mut()
            .clear()
            .extend_pairs(retained)
            .append_pair(
                "sslmode",
                if settings.tls {
                    "verify-full"
                } else {
                    "disable"
                },
            );
        let options = meterstore::DatabaseOptions::new()
            .application_name(Some("histd-registers".into()))
            .max_connections(u32::try_from(settings.pool_size.max(1)).unwrap_or(u32::MAX))
            .acquire_timeout(Duration::seconds(
                i64::try_from(settings.acquire_timeout_s.max(1)).unwrap_or(i64::MAX),
            ));
        let hot = PostgresHot::new(Database::connect_with(url.as_str(), &options).await?);
        let mut table = TableConfig::new(TABLE)
            .value_scale(7)
            .identity_column(Field::new("site_id", DataType::Utf8, false));
        for field in FIELDS {
            table = table.assertion_column(Field::new(field, DataType::Utf8, true));
        }
        let config = table.build()?;
        hot.create_table_for(&config).await?;
        Ok(Self {
            hot,
            config,
            meters,
        })
    }

    pub async fn write(
        &self,
        site: &str,
        meter: &RegisterMeter,
        rows: &BTreeMap<(ems_core::prelude::Slot, OffsetDateTime), &QuarterHour>,
        received: OffsetDateTime,
    ) -> Result<(), StoreError> {
        let mut deliveries = Vec::with_capacity(rows.len());
        let mut days = BTreeSet::new();
        let channel: metering::ObisCode = "1-0:1.8.0".parse().expect("standard import channel");
        for (&(slot, revision), &q) in rows {
            let at = slot.start();
            let interval = MeterInterval::quarter_hour(at, q.grid_draw, QualityFlag::Unknown)
                .map_err(|_| StoreError::InvalidAssertion("invalid quarter-hour quantity"))?
                .with_obis(channel);
            let mut series = MeasurementSeries::new(
                meter.malo,
                Some(channel),
                vec![interval],
                MeasurementSource::DeviceLog {
                    device_id: site.into(),
                    register: None,
                },
                revision,
            );
            series.resolution = Some(Resolution::QUARTER_HOUR);
            let micros = i64::try_from(revision.unix_timestamp_nanos() / 1000).map_err(|_| {
                StoreError::InvalidAssertion("revision exceeds microsecond timestamp range")
            })?;
            let version = Version::new(u128::from(micros.cast_unsigned() ^ (1 << 63)))?;
            let scope = VersionScope::for_interval(meter.operator.to_string(), at, Sparte::Strom)?;
            let mut stored =
                StoredSeries::new(series, ScopedVersion::new(scope, version), received)
                    .with_extra("site_id", ScalarValue::Utf8(Some(site.into())));
            for (name, value) in FIELDS.into_iter().zip([
                Some(q.grid_feed_in),
                Some(q.device_consumption),
                Some(q.device_generation),
                q.storage_consumption,
                q.storage_generation,
                q.anzulegender_wert,
                q.spot_price,
            ]) {
                stored = stored.with_extra(
                    name,
                    ScalarValue::Utf8(value.map(|v| v.normalize().to_string())),
                );
            }
            deliveries.push(stored);
            days.insert(at.to_offset(time::UtcOffset::UTC).date());
        }
        let batch = meterstore::encode::to_record_batch_for(&deliveries, &self.config).map_err(
            |e| match e {
                meterstore::Error::Encode { .. } => StoreError::InvalidAssertion(
                    "register quantity exceeds the MeterStore table precision",
                ),
                other => other.into(),
            },
        )?;
        for day in days {
            let start = day.midnight().assume_utc();
            self.hot
                .ensure_partitions(TABLE, start, start + Duration::DAY, Duration::DAY)
                .await?;
        }
        self.hot
            .append(TABLE, &self.config.merge_key(), &[batch])
            .await
            .map_err(|e| match e {
                meterstore::Error::IntegrityViolation { .. } => StoreError::ConflictingAssertion(
                    "register version conflicts with an existing assertion".into(),
                ),
                other => other.into(),
            })?;
        Ok(())
    }
}
