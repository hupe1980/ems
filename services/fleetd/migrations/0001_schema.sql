-- What survives a restart of `fleetd`.
--
-- The configured half of the fleet — which sites exist, their enrolment secrets,
-- the configuration each should run and the signature over it — is *not* here.
-- That is declared in the daemon's own configuration, which is the record of
-- what an operator intends, and duplicating it into a database gives two answers
-- to one question.
--
-- What is here is the half only the running fleet knows: the credential a box
-- was issued, and what it has said since. Both are facts about the world rather
-- than about the operator's intent, and neither can be re-derived — a lost token
-- is a fleet of boxes holding a credential nothing recognises.
--
-- One row per box that has enrolled.
--
-- The row *is* the single-use property: `site` is the primary key, so a second
-- enrolment attempt collides with the first rather than being refused by a map
-- that a restart emptied.
CREATE TABLE IF NOT EXISTS enrolment (
    site        TEXT        PRIMARY KEY,
    -- The credential the box presents from now on.
    token       TEXT        NOT NULL UNIQUE,
    enrolled_at TIMESTAMPTZ NOT NULL
);

-- What each box last said it is running.
--
-- Separate from `enrolment` because it changes on every report and the
-- enrolment never changes at all. A row appears only once a box has reported:
-- "has not said yet" and "is on version zero" are different facts, and a
-- default would collapse them.
CREATE TABLE IF NOT EXISTS running (
    site            TEXT        PRIMARY KEY REFERENCES enrolment(site) ON DELETE CASCADE,
    running_version TEXT        NOT NULL,
    last_seen       TIMESTAMPTZ NOT NULL
);

-- Answering "which of my boxes have gone quiet" without reading every row.
CREATE INDEX IF NOT EXISTS running_by_last_seen ON running (last_seen);

-- Check-in clocks use whole Unix seconds, matching gateway wire precision.
CREATE TABLE IF NOT EXISTS gateway_checkin (
    site TEXT NOT NULL REFERENCES enrolment(site) ON DELETE CASCADE,
    sent_at BIGINT NOT NULL,
    received_at BIGINT NOT NULL,
    feeder_at BIGINT,
    report_at BIGINT,
    PRIMARY KEY (site, sent_at)
);

-- Simulation receipts have no foreign key into the operational release feed.
CREATE TABLE rehearsal (
    id TEXT PRIMARY KEY,
    canary TEXT NOT NULL REFERENCES enrolment(site) ON DELETE CASCADE,
    cohort TEXT[] NOT NULL,
    component TEXT NOT NULL,
    candidate TEXT NOT NULL CHECK (candidate ~ '^[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    initial_boot TEXT,
    verdict TEXT CHECK (verdict IN ('verified','wrong-target','rejected')),
    boot TEXT,
    running TEXT,
    ready BOOLEAN,
    rollback BOOLEAN,
    sent_at BIGINT,
    observed_at TIMESTAMPTZ,
    conflict BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX rehearsal_by_site ON rehearsal (canary, created_at DESC);
