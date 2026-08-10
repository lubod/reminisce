-- Time-series store for the in-app 1D/30D/90D charts.
-- Sampled by metrics_collector every ~15s; down-sampled at query time via date_bin.
CREATE TABLE IF NOT EXISTS metric_samples (
    ts    TIMESTAMPTZ      NOT NULL,
    name  TEXT             NOT NULL,
    value DOUBLE PRECISION NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_metric_samples_name_ts ON metric_samples (name, ts);
