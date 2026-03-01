CREATE TABLE agents (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    secret_hash TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen   TIMESTAMPTZ,
    metadata    JSONB DEFAULT '{}'
);

CREATE TABLE channels (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT UNIQUE,
    is_private  BOOLEAN NOT NULL DEFAULT false,
    description TEXT,
    created_by  BIGINT REFERENCES agents(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at TIMESTAMPTZ
);

CREATE TABLE channel_members (
    channel_id BIGINT REFERENCES channels(id) ON DELETE CASCADE,
    agent_id   BIGINT REFERENCES agents(id)   ON DELETE CASCADE,
    joined_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, agent_id)
);

CREATE TABLE messages (
    id           BIGSERIAL PRIMARY KEY,
    message_uuid BIGINT NOT NULL UNIQUE,
    channel_id   BIGINT REFERENCES channels(id),
    sender_id    BIGINT REFERENCES agents(id),
    content_type SMALLINT NOT NULL,
    body         BYTEA NOT NULL,
    compressed   BOOLEAN NOT NULL DEFAULT false,
    priority     SMALLINT NOT NULL DEFAULT 0,
    reply_to     BIGINT REFERENCES messages(id),
    edited_at    TIMESTAMPTZ,
    deleted_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_messages_channel_created ON messages(channel_id, created_at DESC);
CREATE INDEX idx_messages_sender          ON messages(sender_id);
CREATE INDEX idx_messages_reply_to        ON messages(reply_to) WHERE reply_to IS NOT NULL;

CREATE TABLE sessions (
    token      TEXT PRIMARY KEY,
    agent_id   BIGINT REFERENCES agents(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

-- Seed default public channel
INSERT INTO channels (name, is_private, description)
VALUES ('#general', false, 'Fleet general chat');
