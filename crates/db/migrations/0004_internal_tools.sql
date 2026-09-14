CREATE TABLE IF NOT EXISTS internal_tools (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    required_permission TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
