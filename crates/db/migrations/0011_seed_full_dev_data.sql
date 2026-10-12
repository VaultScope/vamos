-- 0011_seed_full_dev_data.sql
-- Comprehensive seed data for VaultScope Full Stack development and test environments

-- 1. Ensure Roles Exist
UPDATE roles SET id = '66f01fc4-79c7-4456-986b-117a762389f2' WHERE name = 'Owner' OR name = 'Super Admin';

INSERT INTO roles (id, name, permissions, mapped_group) VALUES
    ('66f01fc4-79c7-4456-986b-117a762389f2', 'Owner', ARRAY['*'], 'vs-admin'),
    ('0ca7a95e-5bf1-4519-b3f6-0e207ff76a3c', 'Support', ARRAY['customers.view','tickets.view','tickets.reply','tickets.manage','services.view'], 'vs-support'),
    ('aa1efaad-b5bd-4548-a6ec-41b92fbac2b2', 'Billing', ARRAY['customers.view','invoices.view','invoices.create','invoices.edit','invoices.delete','billing.view','billing.manage','tax.manage','coupons.manage'], 'vs-billing'),
    ('332da0e5-a529-4666-b247-6d35d3f71b4a', 'Technical', ARRAY['services.view','services.power','services.reinstall','services.manage','connectors.manage','jobs.view'], 'vs-technical')
ON CONFLICT (name) DO UPDATE SET permissions = EXCLUDED.permissions;

-- 2. Staff Members (matching E2E and RBAC test personas)
INSERT INTO staff (id, external_id, username, first_name, last_name, email, email_verified, role_id) VALUES
    ('8fe1b513-a189-4e69-8e12-684e1ef973a3', 'admin1_ext_id', 'admin1', 'Admin', 'One', 'admin1@test.local', true, '66f01fc4-79c7-4456-986b-117a762389f2'),
    ('e8f5fd7a-9e99-4b89-b37e-aece2d3f3f50', 'billing1_ext_id', 'billing1', 'Billing', 'One', 'billing1@test.local', true, 'aa1efaad-b5bd-4548-a6ec-41b92fbac2b2'),
    ('6af11db2-ccc8-44c5-a5dd-4f0ab165966e', 'support1_ext_id', 'support1', 'Support', 'One', 'support1@test.local', true, '0ca7a95e-5bf1-4519-b3f6-0e207ff76a3c'),
    ('dbf68deb-8793-4349-bff5-b761914fed49', 'tech1_ext_id', 'tech1', 'Tech', 'One', 'tech1@test.local', true, '332da0e5-a529-4666-b247-6d35d3f71b4a')
ON CONFLICT (email) DO NOTHING;

-- Also update existing staff if present to ensure verified status
UPDATE staff SET email_verified = true WHERE email IN ('admin1@test.local', 'billing1@test.local', 'support1@test.local', 'tech1@test.local');

-- 3. Customers
INSERT INTO customers (id, external_id, name, email, phone, company, address, city, country, vat_id, status, email_verified) VALUES
    ('01a09b2e-0532-7403-86db-a10464205764', 'customer1_ext_id', 'Anton Schmidt', 'customer1@test.local', '+4915172210043', 'VaultScope - Customer', 'Gottlob-Spiess-Straße 2', 'Sachsenheim', 'Germany', '', 'active', true),
    ('01a081be-0b06-7483-8ef8-964179d3fc5c', 'customer2_ext_id', 'Max Mustermann', 'max@example.com', '+49123456789', 'Mustermann Cloud GmbH', 'Musterstrasse 1', 'Berlin', 'Germany', 'DE123456789', 'active', true)
ON CONFLICT (email) DO UPDATE SET status = 'active', email_verified = true;

-- 4. Connectors
INSERT INTO connectors (id, name, provider, status)
VALUES ('00000000-0000-0000-0000-000000000001', 'Default Hetzner', 'hetzner_cloud', 'connected')
ON CONFLICT (name) DO UPDATE SET status = 'connected';

-- 5. Products (Standardize category to lowercase 'vps' and 'dedicated')
UPDATE products SET category = 'vps' WHERE name = 'CX22';

INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden, connector_id) VALUES
    ('00000000-0000-0000-0000-000000000002', 'CX22', 'vps', 'hetzner_cloud', 'vps', '{"server_type": "cx22", "location": "fsn1", "image": "ubuntu-24.04", "cpu": 2, "ram": "4 GB", "disk": "40 GB NVMe"}', 2.50, 4.00, 0, -1, 0, 'monthly', false, '00000000-0000-0000-0000-000000000001'),
    ('00000000-0000-0000-0000-000000000003', 'CX32', 'vps', 'hetzner_cloud', 'vps', '{"server_type": "cx32", "location": "fsn1", "image": "ubuntu-24.04", "cpu": 4, "ram": "8 GB", "disk": "80 GB NVMe"}', 5.50, 8.50, 0, -1, 0, 'monthly', false, '00000000-0000-0000-0000-000000000001'),
    ('00000000-0000-0000-0000-000000000004', 'CX42', 'vps', 'hetzner_cloud', 'vps', '{"server_type": "cx42", "location": "fsn1", "image": "ubuntu-24.04", "cpu": 8, "ram": "16 GB", "disk": "160 GB NVMe"}', 10.00, 16.00, 0, -1, 0, 'monthly', false, '00000000-0000-0000-0000-000000000001'),
    ('00000000-0000-0000-0000-000000000005', 'AX41-NVMe', 'dedicated', 'hetzner_cloud', 'dedicated', '{"server_type": "ax41", "location": "fsn1", "cpu": "AMD Ryzen 5 3600", "ram": "64 GB DDR4", "disk": "2x 512 GB NVMe"}', 34.00, 44.00, 39.00, 5, 0, 'monthly', false, '00000000-0000-0000-0000-000000000001')
ON CONFLICT (id) DO UPDATE SET 
    category = EXCLUDED.category,
    price = EXCLUDED.price,
    specs = EXCLUDED.specs,
    connector_id = EXCLUDED.connector_id;

-- 6. Departments
INSERT INTO departments (id, name, mailbox, default_assignee_id) VALUES
    ('00000000-0000-0000-0000-000000000010', 'Technical Support', 'support@vaultscope.de', '6af11db2-ccc8-44c5-a5dd-4f0ab165966e'),
    ('00000000-0000-0000-0000-000000000011', 'Billing & Accounts', 'billing@vaultscope.de', 'e8f5fd7a-9e99-4b89-b37e-aece2d3f3f50'),
    ('00000000-0000-0000-0000-000000000012', 'Abuse & Security', 'abuse@vaultscope.de', 'dbf68deb-8793-4349-bff5-b761914fed49')
ON CONFLICT (name) DO NOTHING;

-- 7. Mailboxes
INSERT INTO mailboxes (id, name, host, port, user_name, password, is_default) VALUES
    ('00000000-0000-0000-0000-000000000020', 'Default SMTP', 'mailpit', 1025, 'noreply@vaultscope.de', 'mock-password', true)
ON CONFLICT DO NOTHING;

-- Link default mailbox to existing templates
UPDATE email_templates SET mailbox_id = '00000000-0000-0000-0000-000000000020' WHERE mailbox_id IS NULL;

-- 8. Tax Rates
INSERT INTO tax_rates (id, name, country, rate) VALUES
    ('00000000-0000-0000-0000-000000000030', 'MwSt 19%', 'DE', 19.00),
    ('00000000-0000-0000-0000-000000000031', 'EU Standard 19%', 'EU', 19.00),
    ('00000000-0000-0000-0000-000000000032', 'Reverse Charge 0%', 'OTHER', 0.00)
ON CONFLICT DO NOTHING;

-- 9. Key-Value Settings
INSERT INTO settings (key, value) VALUES
    ('brand_name', '"VaultScope"'),
    ('currency', '"EUR"'),
    ('support_email', '"support@vaultscope.de"'),
    ('stripe_test_mode_enabled', 'true'),
    ('stripe_public_key', '"pk_test_vaultscope_mock"')
ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value;

-- 10. Coupons
INSERT INTO coupons (id, code, discount_type, discount_value, usage_limit, usage_count, status) VALUES
    ('00000000-0000-0000-0000-000000000040', 'WELCOME10', 'percentage', 10.00, 100, 0, 'active'),
    ('00000000-0000-0000-0000-000000000041', 'DEV50', 'fixed', 5.00, 50, 0, 'active')
ON CONFLICT (code) DO NOTHING;

-- 11. Internal Tools
INSERT INTO internal_tools (id, name, url, required_permission) VALUES
    ('00000000-0000-0000-0000-000000000050', 'Documentation', 'https://docs.vaultscope.de', '*'),
    ('00000000-0000-0000-0000-000000000051', 'Mailpit SMTP Server', 'http://localhost:8025', '*')
ON CONFLICT DO NOTHING;

-- 12. Demo Operational Data: Services, Invoices, Tickets
INSERT INTO services (id, customer_id, product_id, connector_id, name, status, provider_resource_id, ip, hostname, price) VALUES
    ('00000000-0000-0000-0000-000000000060', '01a09b2e-0532-7403-86db-a10464205764', '00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'web-prod-01', 'running', 'srv-hetzner-1001', '195.201.12.34', 'vps-01.vaultscope.local', 4.00)
ON CONFLICT (id) DO NOTHING;

INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, paid_at) VALUES
    ('00000000-0000-0000-0000-000000000070', 'INV-2026-0001', '01a09b2e-0532-7403-86db-a10464205764', 'paid', 4.00, 19.00, 0.76, 4.76, CURRENT_DATE - INTERVAL '5 days', CURRENT_DATE + INTERVAL '25 days', now())
ON CONFLICT (id) DO NOTHING;

INSERT INTO invoice_line_items (id, invoice_id, description, quantity, unit_price, total, service_id, sort_order) VALUES
    ('00000000-0000-0000-0000-000000000071', '00000000-0000-0000-0000-000000000070', 'CX22 Cloud VPS (Monthly)', 1, 4.00, 4.00, '00000000-0000-0000-0000-000000000060', 0)
ON CONFLICT (id) DO NOTHING;

INSERT INTO tickets (id, ticket_number, customer_id, category, subject, status, priority, assignee_id, mailbox, related_service_id) VALUES
    ('00000000-0000-0000-0000-000000000080', 'TICK-1001', '01a09b2e-0532-7403-86db-a10464205764', 'support', 'Welcome to VaultScope Cloud Services', 'open', 'normal', '6af11db2-ccc8-44c5-a5dd-4f0ab165966e', 'support@vaultscope.de', '00000000-0000-0000-0000-000000000060')
ON CONFLICT (id) DO NOTHING;

INSERT INTO ticket_messages (id, ticket_id, author_id, author_type, content, internal) VALUES
    ('00000000-0000-0000-0000-000000000081', '00000000-0000-0000-0000-000000000080', '6af11db2-ccc8-44c5-a5dd-4f0ab165966e', 'staff', 'Hello Anton! Your new CX22 instance is provisioned and running. Please let our team know if you have any questions.', false)
ON CONFLICT (id) DO NOTHING;
