INSERT INTO departure_receipt AS old
    (site, departure, requested_wh, opening_wh, rejoin_wh, closing_wh)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (site) DO UPDATE SET
    departure    = EXCLUDED.departure,
    requested_wh = EXCLUDED.requested_wh,
    opening_wh   = EXCLUDED.opening_wh,
    rejoin_wh    = EXCLUDED.rejoin_wh,
    closing_wh   = CASE WHEN old.departure = EXCLUDED.departure
                        THEN COALESCE(old.closing_wh, EXCLUDED.closing_wh)
                        ELSE EXCLUDED.closing_wh END
WHERE EXCLUDED.departure > old.departure OR (
    EXCLUDED.departure = old.departure
    AND EXCLUDED.requested_wh = old.requested_wh
    AND EXCLUDED.opening_wh = old.opening_wh
    AND EXCLUDED.rejoin_wh = old.rejoin_wh
    AND (old.closing_wh IS NULL OR EXCLUDED.closing_wh IS NULL
         OR old.closing_wh = EXCLUDED.closing_wh)
);
