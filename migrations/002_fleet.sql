-- migrations/002_fleet.sql

CREATE TABLE fleets (
    id         BIGSERIAL PRIMARY KEY,
    name       TEXT    NOT NULL UNIQUE,
    owner_id   BIGINT  NOT NULL REFERENCES agents(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE fleet_shares (
    fleet_id             BIGINT NOT NULL REFERENCES fleets(id),
    shared_with_fleet_id BIGINT NOT NULL REFERENCES fleets(id),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (fleet_id, shared_with_fleet_id)
);

ALTER TABLE agents
    ADD COLUMN fleet_id           BIGINT  REFERENCES fleets(id),
    ADD COLUMN is_human           BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN default_channel_id BIGINT  REFERENCES channels(id),
    ADD COLUMN agent_uuid         UUID    NOT NULL DEFAULT gen_random_uuid();

ALTER TABLE channels
    ADD COLUMN fleet_id BIGINT REFERENCES fleets(id);
