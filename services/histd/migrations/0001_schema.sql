-- histd — the fleet's copy of the § 14a record, across every site.
--
-- Three tables, and the shape of each is decided by the question it has to
-- answer rather than by what was convenient to write.
--
-- `quarter_hour` is the **settlement** record: the meter registers MiSpeL's
-- Abgrenzung (BK 618-25-02) and § 42c's allocation are computed from.
--
-- `control_event` and `compliance_sample` are the **evidence** record of
-- [A1 7.2]: what the network operator asked for, when, what was done about it,
-- and what the connection point actually drew while it lasted. [A1 7.3] says it
-- is kept for two years, which is why `expires_at` is a column rather than a
-- policy somebody remembers to apply.
--
-- # What changed when this stopped being SQLite
--
-- Every quantity below is a `NUMERIC`. On SQLite they were decimal *strings*,
-- because SQLite has no exact numeric type and a settlement that went through a
-- float is a settlement nobody can reproduce (P3). PostgreSQL has one, and
-- `rust_decimal` round-trips through it exactly — so `SUM(grid_draw_kwh)` is now
-- arithmetic the database does rather than 35 040 strings this workspace parses,
-- and a whole class of "the column did not contain a decimal" error stops
-- existing.
--
-- Every instant is a `TIMESTAMPTZ` rather than Unix seconds, so a range query is
-- a range query and `psql` shows an operator a date. The stored value is still
-- an instant and not a wall-clock string: a household's own day boundary is
-- `metering`'s question, and storing a local string here would ask it twice,
-- differently, twice a year.

-- One site's quarter-hour meter registers.
CREATE TABLE IF NOT EXISTS quarter_hour (
    site_id                TEXT        NOT NULL,
    -- The slot's start.
    slot_start             TIMESTAMPTZ NOT NULL,
    grid_draw_kwh          NUMERIC     NOT NULL,
    grid_feed_in_kwh       NUMERIC     NOT NULL,
    device_consumption_kwh NUMERIC     NOT NULL,
    device_generation_kwh  NUMERIC     NOT NULL,
    -- `Z3V¼`/`Z3E¼` — the storage system **alone**, where the box reads its own
    -- meter. `Z2` above is the store and the charge point together, and
    -- Basisfall A4 is precisely the case where the store is separately metered
    -- so that `[MiSpeL A1 (17)A4]` can charge the conversion losses to it rather
    -- than to the household (`[MiSpeL A1 3.2.4]`).
    --
    -- Nullable, and the null means "not separately metered" rather than zero. A
    -- household declared A4 whose store went unread owes its network operator a
    -- refusal, and `hems_grid::mispel` gives it one; a zero here would be a
    -- settlement claiming the battery stood still.
    storage_consumption_kwh NUMERIC,
    storage_generation_kwh  NUMERIC,
    anzulegender_wert_ct   NUMERIC,
    spot_price_ct          NUMERIC,
    -- Producer revision identifies a correction. The historian assigns its own
    -- knowledge time on first acceptance, including for delayed deliveries.
    assertion_at           TIMESTAMPTZ NOT NULL,
    recorded_at            TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (site_id, assertion_at, slot_start)
);

CREATE INDEX IF NOT EXISTS quarter_hour_version
    ON quarter_hour (site_id, slot_start, assertion_at DESC, recorded_at);

-- A retry compares all columns (including nullable prices and registers).
-- Skipping an identical UPDATE preserves the first acceptance time and avoids
-- writing a dead tuple; a mismatch aborts the complete INSERT batch.
CREATE FUNCTION histd_immutable_assertion() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW IS DISTINCT FROM OLD THEN
        RAISE EXCEPTION USING
            ERRCODE = 'HS001',
            MESSAGE = format('slot %s already has different registers at producer revision %s',
                             OLD.slot_start, OLD.assertion_at);
    END IF;
    RETURN NULL;
END;
$$;

CREATE TRIGGER immutable_assertion BEFORE UPDATE ON quarter_hour
    FOR EACH ROW EXECUTE FUNCTION histd_immutable_assertion();

-- Retention, which is a sweep over `slot_start` across every site.
CREATE INDEX IF NOT EXISTS quarter_hour_by_slot ON quarter_hour (slot_start);

-- One § 14a control event, [A1 7.2].
--
-- **A document plus projections.** `document` is the whole `ControlEvent` as
-- `serde` JSON and is the only thing the event is ever *reconstructed* from; the
-- columns beside it exist to be queried, filtered and indexed on, and are
-- derived from the same value in the same statement.
--
-- A column per field with the enums written through `format!("{:?}")` cannot be
-- read back: `Debug` is not a serialisation, nothing promises it round trips,
-- and renaming a variant would silently change what two years of evidence say.
--
-- `JSONB` rather than `TEXT`: it is the same bytes to this daemon and it lets an
-- operator ask a question of the document in `psql` without a client that can
-- parse it — which is what somebody does at two in the morning when the columns
-- beside it turn out not to carry the field they needed.
CREATE TABLE IF NOT EXISTS control_event (
    id                  BIGINT      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    site_id             TEXT        NOT NULL,
    -- The event itself, **without** its samples: those are the table below, so
    -- that each fact has one home and a trace of ten thousand rows is not
    -- re-parsed to answer "how many".
    document            JSONB       NOT NULL,
    -- Projections. Everything below is derivable from `document` and is stored
    -- because a query needs it: the rule to tell an operator's instruction from
    -- the box restraining itself, the instants to window on, the powers and the
    -- flag so a Nachweis does not parse two years of JSON to find one breach.
    rule                TEXT        NOT NULL,
    received_at         TIMESTAMPTZ NOT NULL,
    released_at         TIMESTAMPTZ,
    first_ceiling_w     DOUBLE PRECISION NOT NULL,
    strictest_ceiling_w DOUBLE PRECISION NOT NULL,
    minimum_power_w     DOUBLE PRECISION NOT NULL,
    below_minimum       BOOLEAN     NOT NULL,
    -- Two years from the day it closed, [A1 7.3]. A column rather than a policy:
    -- a retention rule nobody can query is a retention rule nobody can prove.
    expires_at          TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS control_event_by_site
    ON control_event (site_id, received_at);
CREATE INDEX IF NOT EXISTS control_event_by_expiry
    ON control_event (expires_at);
-- The breach list, which is what `obsd` and a network operator ask for and what
-- would otherwise be a sequential scan over two years of every household.
-- Partial, because the rows it answers about are a small fraction of the table.
CREATE INDEX IF NOT EXISTS control_event_below_minimum
    ON control_event (site_id, received_at) WHERE below_minimum;

-- The minute-resolution trace of what the connection point drew, [A1 7.2].
CREATE TABLE IF NOT EXISTS compliance_sample (
    event_id      BIGINT      NOT NULL REFERENCES control_event(id) ON DELETE CASCADE,
    at            TIMESTAMPTZ NOT NULL,
    netzwirksam_w DOUBLE PRECISION NOT NULL,
    ceiling_w     DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (event_id, at)
);

-- A site's storage identity survives restarts and is immutable after commissioning.
CREATE TABLE register_meter (
    site_id TEXT PRIMARY KEY,
    malo TEXT NOT NULL,
    operator_id TEXT NOT NULL
);
