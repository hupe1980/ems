//! Check production window, evidence and retention query plans against indexed
//! PostgreSQL data. Each receipt selects the latest source revision visible at
//! its historian cutoff. Custom plans retain the site's actual time bounds.

use ems_service::testdb::Postgres;

/// Enough rows, across enough sites, that a sequential scan is not the cheapest
/// plan by accident. A planner given a hundred rows will read them all whatever
/// the SQL says, and a test that passed on that would be testing nothing.
const SITES: i32 = 50;
const ROWS: i32 = 40_000;

async fn loaded() -> Postgres {
    let fixture = Postgres::start(histd::store::MIGRATIONS).await;
    let client = fixture.db.get().await.expect("a connection");
    client
        .batch_execute(&format!(
            "INSERT INTO quarter_hour
             SELECT 'site-' || (g % {SITES}),
                    '2025-01-01Z'::timestamptz + ((g / {SITES}) || ' minutes')::interval,
                    1, 1, 1, 1, NULL, NULL, 1, 1,
                    '2025-01-01Z'::timestamptz + (g || ' seconds')::interval,
                    '2025-01-02Z'::timestamptz + (g || ' seconds')::interval
             FROM generate_series(1, {ROWS}) g
             ON CONFLICT DO NOTHING;
             INSERT INTO control_event
                 (site_id, document, rule, received_at, first_ceiling_w,
                  strictest_ceiling_w, minimum_power_w, below_minimum, expires_at)
             SELECT 'site-' || (g % {SITES}), '{{}}'::jsonb, 'lpc',
                    '2025-01-01Z'::timestamptz + ((g / {SITES}) || ' hours')::interval,
                    4200, 4200, 10500, false, 'infinity'::timestamptz
             FROM generate_series(1, 5000) g;
             ANALYZE quarter_hour;
             ANALYZE control_event;"
        ))
        .await
        .expect("a loaded table");
    fixture
}

/// The plan a **serving** connection produces for `sql`.
///
/// Prepared and executed exactly as `tokio-postgres` does it, on a connection
/// from the pool — so the session settings under test are the ones the daemon
/// actually runs with rather than a psql default.
async fn plan_of(fixture: &Postgres, name: &str, types: &str, sql: &str, args: &str) -> String {
    let client = fixture.db.get().await.expect("a connection");
    client
        .batch_execute(&format!("PREPARE {name}({types}) AS {sql};"))
        .await
        .expect("the statement prepares");
    let rows = client
        .query(&format!("EXPLAIN (COSTS OFF) EXECUTE {name}({args})"), &[])
        .await
        .expect("a plan");
    rows.iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn one_sites_window_keeps_every_bound_in_its_index_condition() {
    let fixture = loaded().await;
    let plan = plan_of(
        &fixture,
        "window_q",
        "text, timestamptz, timestamptz, timestamptz",
        histd::store::REGISTER_WINDOW,
        "'site-3', '2025-01-01T00:05Z', '2025-01-01T00:20Z', '2025-01-02T00:10Z'",
    )
    .await;

    assert!(
        plan.contains("quarter_hour_version"),
        "the window query left its key index:\n{plan}"
    );
    let range = plan
        .lines()
        .find(|line| line.contains("Index Cond:"))
        .expect("range scan");
    for bound in [
        "site_id =",
        "slot_start >=",
        "slot_start <",
        "recorded_at <=",
    ] {
        assert!(
            range.contains(bound),
            "{bound} moved outside the index condition:\n{plan}"
        );
    }
}

#[tokio::test]
async fn one_sites_events_are_an_index_scan() {
    // The query behind a Nachweis `[A1 7.2]`.
    let fixture = loaded().await;
    let plan = plan_of(
        &fixture,
        "events_q",
        "text, timestamptz, timestamptz",
        "SELECT id FROM control_event
         WHERE site_id = $1 AND received_at >= $2 AND received_at < $3
         ORDER BY received_at, id",
        "'site-3', '2025-01-01Z', '2025-01-02Z'",
    )
    .await;

    assert!(
        plan.contains("control_event_by_site"),
        "the Nachweis query left its index:\n{plan}"
    );
    assert!(
        !plan.contains("Seq Scan"),
        "a Nachweis must never be a sequential scan:\n{plan}"
    );
}

#[tokio::test]
async fn the_retention_sweep_does_not_read_the_whole_table() {
    // `[A1 7.3]`'s sweep runs daily over every household and deletes **one day**
    // of a two-year record. Without an index on `slot_start` that is a full scan
    // of the fleet's whole settlement record to remove a fraction of a per cent
    // of it — which is why that index exists even though it competes with the
    // key index on the query above.
    //
    // The cutoff matters to what this proves: asking to delete most of the table
    // correctly plans as a sequential scan, and a test written that way would
    // fail for the right reason and the wrong cause.
    let fixture = loaded().await;
    let client = fixture.db.get().await.expect("a connection");
    let rows = client
        .query(
            "EXPLAIN (COSTS OFF) DELETE FROM quarter_hour WHERE slot_start <= '2025-01-01 00:30:00Z'",
            &[],
        )
        .await
        .expect("a plan");
    let plan: String = rows
        .iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains("quarter_hour_by_slot"),
        "the retention sweep left its index:\n{plan}"
    );
}

#[tokio::test]
async fn a_serving_connection_replans_with_its_parameters() {
    // The setting that makes the two queries above optimal *together*: with both
    // indexes present and no parameter values, a generic plan cannot tell which
    // is more selective. `prepare_session` forces a custom plan on every
    // connection the pool hands out, which is the whole reason a window query
    // and an export can be one statement.
    let fixture = Postgres::start(histd::store::MIGRATIONS).await;
    let client = fixture.db.get().await.expect("a connection");
    let mode: String = client
        .query_one("SHOW plan_cache_mode", &[])
        .await
        .expect("the session setting")
        .get(0);
    assert_eq!(mode, "force_custom_plan");
}
