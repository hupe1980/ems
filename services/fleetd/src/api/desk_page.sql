-- Use one clock and one snapshot for the queue, its counts and its page.
WITH latest AS (
    SELECT e.site, c.received_at, c.feeder_at, c.report_at,
           floor(extract(epoch FROM statement_timestamp()))::bigint AS now_s
    FROM enrolment e
    LEFT JOIN LATERAL (
        SELECT received_at, feeder_at, report_at FROM gateway_checkin
        WHERE site = e.site ORDER BY sent_at DESC LIMIT 1
    ) c ON true
    WHERE ($1::text[] IS NULL OR e.site = ANY($1))
      AND ($2::text IS NULL OR e.site = $2)
      AND strpos(lower(e.site), lower($3::text)) > 0
), ranked AS (
    SELECT *, CASE
        WHEN received_at IS NULL THEN 4
        WHEN received_at > now_s THEN 1
        WHEN now_s - received_at > $7 THEN 0
        WHEN feeder_at IS NULL OR now_s - feeder_at NOT BETWEEN 0 AND $8 THEN 2
        WHEN report_at IS NULL OR now_s - report_at NOT BETWEEN 0 AND $9 THEN 3
        ELSE 5 END AS priority
    FROM latest
), matches AS (
    SELECT * FROM ranked
    WHERE CASE $4::text WHEN 'attention' THEN priority < 5 WHEN 'current' THEN priority = 5
          WHEN 'cohort' THEN site IN (
              SELECT unnest(cohort) FROM rehearsal WHERE id = $5
                AND ($1 IS NULL OR canary = ANY($1))
          ) ELSE true END
), counts AS (
    SELECT count(*) AS total, count(*) FILTER (WHERE priority < 5) AS attention,
           (SELECT count(*) FROM matches) AS matched FROM ranked
), bounds AS (
    SELECT *, least($6::bigint, greatest(matched - 1, 0) / $10) AS page FROM counts
)
SELECT total, attention, matched, page, item.*,
       h.sent_at, h.received_at AS receipt_at, h.feeder_at AS sample_at, h.report_at AS closed_at
FROM bounds
LEFT JOIN LATERAL (
    SELECT * FROM matches ORDER BY priority, site COLLATE "C"
    LIMIT $10::bigint OFFSET bounds.page * $10
) item ON true
LEFT JOIN LATERAL (
    SELECT sent_at, received_at, feeder_at, report_at FROM gateway_checkin
    WHERE $2 IS NOT NULL AND site = item.site ORDER BY sent_at DESC LIMIT 32
) h ON true
ORDER BY item.priority, item.site COLLATE "C", h.sent_at DESC;
