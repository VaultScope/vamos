ALTER TABLE products ADD COLUMN connector_id UUID REFERENCES connectors(id) ON DELETE SET NULL;
