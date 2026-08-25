-- Seed dummy data
INSERT INTO connectors (id, name, provider, status)
VALUES ('00000000-0000-0000-0000-000000000001', 'Default Hetzner', 'hetzner_cloud', 'connected')
ON CONFLICT (name) DO NOTHING;

INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden)
VALUES ('00000000-0000-0000-0000-000000000002', 'CX22', 'VPS', 'hetzner_cloud', 'vps', '{"server_type": "cx22", "location": "fsn1", "image": "ubuntu-22.04"}', 2.50, 4.00, 0, -1, 0, 'monthly', false)
ON CONFLICT (id) DO NOTHING;
