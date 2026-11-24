-- The releases the panel has seen on offer, and what it did about each: told the administrators, tried the
-- automatic update. A restart (an update ends in one) does not start the waiting again.
CREATE TABLE panel_updates (
    version             text        PRIMARY KEY CHECK (version ~ '^[0-9]+\.[0-9]+\.[0-9]+$'),
    first_seen_at       timestamptz NOT NULL DEFAULT now(),
    notified_at         timestamptz,
    auto_attempted_at   timestamptz,
    failure_notified_at timestamptz
);
