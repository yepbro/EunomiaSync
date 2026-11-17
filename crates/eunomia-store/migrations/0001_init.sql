-- Initialize the EunomiaSync application database schema.
-- `meta` holds lightweight key/value application metadata (schema markers, feature flags).
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
