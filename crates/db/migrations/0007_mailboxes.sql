CREATE TABLE mailboxes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    port INT NOT NULL,
    user_name TEXT NOT NULL,
    password TEXT NOT NULL,
    is_default BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMP NOT NULL DEFAULT now()
);

ALTER TABLE email_templates ADD COLUMN mailbox_id UUID REFERENCES mailboxes(id);
