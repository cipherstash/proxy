
CREATE TABLE blah (
    id bigint GENERATED ALWAYS AS IDENTITY,
    t TEXT,
    j JSONB,
    vtha JSONB,
    PRIMARY KEY(id)
);