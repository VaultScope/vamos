-- Migration 0036: ReliableSite Order & Inventory Catalog Schema + Real Netherlands (NL) Seed Data
-- Source: http://api.reliablesite.net/Inventory.svc/Inventory (ServersList, AddonGroupsList, AddonsList)

CREATE TABLE IF NOT EXISTS reliablesite_servers (
    product_id INTEGER PRIMARY KEY,
    description TEXT NOT NULL,
    data_center TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT '',
    stock INTEGER NOT NULL DEFAULT 0,
    is_preorder BOOLEAN NOT NULL DEFAULT false,
    setup_1_hour NUMERIC(12,4) NOT NULL DEFAULT 0,
    setup_1_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    setup_3_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    setup_6_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    setup_12_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    setup_24_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    setup_36_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    recurring_1_hour NUMERIC(12,4) NOT NULL DEFAULT -1,
    recurring_1_month NUMERIC(12,4) NOT NULL DEFAULT -1,
    recurring_3_month NUMERIC(12,4) NOT NULL DEFAULT -1,
    recurring_6_month NUMERIC(12,4) NOT NULL DEFAULT -1,
    recurring_12_month NUMERIC(12,4) NOT NULL DEFAULT -1,
    recurring_24_month NUMERIC(12,4) NOT NULL DEFAULT -1,
    recurring_36_month NUMERIC(12,4) NOT NULL DEFAULT -1,
    vaultscope_product_id UUID REFERENCES products(id) ON DELETE SET NULL,
    reseller_promo_code TEXT,
    synced_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS reliablesite_addon_groups (
    group_id INTEGER PRIMARY KEY,
    description TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    required BOOLEAN NOT NULL DEFAULT true
);

CREATE TABLE IF NOT EXISTS reliablesite_product_addon_groups (
    product_id INTEGER NOT NULL REFERENCES reliablesite_servers(product_id) ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES reliablesite_addon_groups(group_id) ON DELETE CASCADE,
    sort_order INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (product_id, group_id)
);

CREATE TABLE IF NOT EXISTS reliablesite_addons (
    addon_id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES reliablesite_addon_groups(group_id) ON DELETE CASCADE,
    addon_description TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    setup_1_hour NUMERIC(12,4) NOT NULL DEFAULT 0,
    setup_1_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    setup_3_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    setup_6_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    setup_12_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    setup_24_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    setup_36_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    recurring_1_hour NUMERIC(12,4) NOT NULL DEFAULT 0,
    recurring_1_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    recurring_3_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    recurring_6_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    recurring_12_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    recurring_24_month NUMERIC(12,4) NOT NULL DEFAULT 0,
    recurring_36_month NUMERIC(12,4) NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_rs_servers_dc ON reliablesite_servers(data_center);
CREATE INDEX IF NOT EXISTS idx_rs_servers_product ON reliablesite_servers(vaultscope_product_id);
CREATE INDEX IF NOT EXISTS idx_rs_addons_group ON reliablesite_addons(group_id);

-- Seed Netherlands Dedicated Server Products
INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden)
VALUES ('10000000-0000-4000-8000-000000000424', 'Pre-Order - AMD Ryzen 9950X 256GB (EU)', 'dedicated', 'reliablesite', '424', '{"reliablesite_product_id": 424, "datacenter": "NL", "location": "Amsterdam, NL", "server_type": "424", "cpu": "AMD Ryzen 9950X", "ram": "256 GB DDR5 RAM", "disk": "2 x 4 TB NVMe (PCIe 4.0)", "uplink": "1 Gbps Unmetered", "traffic": "Unmetered", "ipmi": "Dedicated KVM Access with Virtual Media", "preorder": true, "detail": "AMD Ryzen 9950X<br />\r\n256 GB DDR5 RAM<br />\r\n2 x 4 TB NVMe (PCIe 4.0)<br />\r\nFirst Available EU Data Center<br />\r\nUp to 2 Week Setup Time"}'::jsonb, 599.00, 599.00, 0.00, 100, 0, 'monthly', false)
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    category = EXCLUDED.category,
    provider = EXCLUDED.provider,
    target = EXCLUDED.target,
    specs = EXCLUDED.specs,
    cost = EXCLUDED.cost,
    setup_fee = EXCLUDED.setup_fee,
    stock = EXCLUDED.stock,
    updated_at = now();
INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden)
VALUES ('10000000-0000-4000-8000-000000000425', 'Pre-Order - AMD Epyc 4545P 256GB (EU)', 'dedicated', 'reliablesite', '425', '{"reliablesite_product_id": 425, "datacenter": "NL", "location": "Amsterdam, NL", "server_type": "425", "cpu": "AMD Epyc 4545P", "ram": "256 GB DDR5 RAM", "disk": "4 TB NVMe (PCIe 4.0)", "uplink": "1 Gbps Unmetered", "traffic": "Unmetered", "ipmi": "Dedicated KVM Access with Virtual Media", "preorder": true, "detail": "AMD Epyc 4545P<br />\r\n256 GB DDR5 RAM<br />\r\n4 TB NVMe (PCIe 4.0)<br />\r\nFirst Available EU Data Center<br />\r\nUp to 2 Week Setup Time"}'::jsonb, 549.00, 549.00, 0.00, 0, 0, 'monthly', false)
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    category = EXCLUDED.category,
    provider = EXCLUDED.provider,
    target = EXCLUDED.target,
    specs = EXCLUDED.specs,
    cost = EXCLUDED.cost,
    setup_fee = EXCLUDED.setup_fee,
    stock = EXCLUDED.stock,
    updated_at = now();
INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden)
VALUES ('10000000-0000-4000-8000-000000000426', 'Rapid Deploy Server - AMD Ryzen 9950X 256GB (NL)', 'dedicated', 'reliablesite', '426', '{"reliablesite_product_id": 426, "datacenter": "NL", "location": "Amsterdam, NL", "server_type": "426", "cpu": "AMD Ryzen 9950X", "ram": "256GB DDR5 RAM", "disk": "2 x 4 TB NVMe (PCIe 4.0)", "uplink": "1 Gbps Unmetered", "traffic": "Unmetered", "ipmi": "Dedicated KVM Access with Virtual Media", "preorder": false, "detail": "AMD Ryzen 9950X<br />\r\n256GB DDR5 RAM<br />\r\n2 x 4 TB NVMe (PCIe 4.0)<br />\r\nAmsterdam, NL Data Center"}'::jsonb, 599.00, 599.00, 0.00, 5, 0, 'monthly', false)
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    category = EXCLUDED.category,
    provider = EXCLUDED.provider,
    target = EXCLUDED.target,
    specs = EXCLUDED.specs,
    cost = EXCLUDED.cost,
    setup_fee = EXCLUDED.setup_fee,
    stock = EXCLUDED.stock,
    updated_at = now();
INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden)
VALUES ('10000000-0000-4000-8000-000000000427', 'Rapid Deploy Server - AMD Epyc 4545P 256GB (NL)', 'dedicated', 'reliablesite', '427', '{"reliablesite_product_id": 427, "datacenter": "NL", "location": "Amsterdam, NL", "server_type": "427", "cpu": "AMD Epyc 4545P", "ram": "256 GB DDR5 RAM", "disk": "4 TB NVMe", "uplink": "1 Gbps Unmetered", "traffic": "Unmetered", "ipmi": "Dedicated KVM Access with Virtual Media", "preorder": false, "detail": "AMD Epyc 4545P<br />\r\n256 GB DDR5 RAM<br />\r\n4 TB NVMe<br />\r\nAmsterdam, NL Data Center"}'::jsonb, 549.00, 549.00, 0.00, 8, 0, 'monthly', false)
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    category = EXCLUDED.category,
    provider = EXCLUDED.provider,
    target = EXCLUDED.target,
    specs = EXCLUDED.specs,
    cost = EXCLUDED.cost,
    setup_fee = EXCLUDED.setup_fee,
    stock = EXCLUDED.stock,
    updated_at = now();
INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden)
VALUES ('10000000-0000-4000-8000-000000000428', 'Rapid Deploy Server - AMD Epyc 4545P 128GB (NL)', 'dedicated', 'reliablesite', '428', '{"reliablesite_product_id": 428, "datacenter": "NL", "location": "Amsterdam, NL", "server_type": "428", "cpu": "AMD Epyc 4545P", "ram": "128 GB DDR5 RAM", "disk": "2 TB NVMe", "uplink": "1 Gbps Unmetered", "traffic": "Unmetered", "ipmi": "Dedicated KVM Access with Virtual Media", "preorder": false, "detail": "AMD Epyc 4545P<br />\r\n128 GB DDR5 RAM<br />\r\n2 TB NVMe<br />\r\nAmsterdam, NL Data Center"}'::jsonb, 299.00, 299.00, 0.00, 7, 0, 'monthly', false)
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    category = EXCLUDED.category,
    provider = EXCLUDED.provider,
    target = EXCLUDED.target,
    specs = EXCLUDED.specs,
    cost = EXCLUDED.cost,
    setup_fee = EXCLUDED.setup_fee,
    stock = EXCLUDED.stock,
    updated_at = now();
INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden)
VALUES ('10000000-0000-4000-8000-000000000444', 'Rapid Deploy Server - AMD Epyc 4545P GPU (NL)', 'dedicated', 'reliablesite', '444', '{"reliablesite_product_id": 444, "datacenter": "NL", "location": "Amsterdam, NL", "server_type": "444", "cpu": "AMD Epyc 4545P", "ram": "128 GB DDR5 RAM", "disk": "1TB NVMe", "uplink": "1 Gbps Unmetered", "traffic": "Unmetered", "ipmi": "Dedicated KVM Access with Virtual Media", "preorder": false, "detail": "AMD Epyc 4545P<br />\r\n128 GB DDR5 RAM<br />\r\n1TB NVMe<br />\r\nNVIDIA RTX PRO 4500 Blackwell GPU<br />\r\nAmsterdam, Netherlands Data Center", "gpu": "NVIDIA RTX PRO 4500 Blackwell GPU"}'::jsonb, 999.00, 999.00, 0.00, 0, 0, 'monthly', false)
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    category = EXCLUDED.category,
    provider = EXCLUDED.provider,
    target = EXCLUDED.target,
    specs = EXCLUDED.specs,
    cost = EXCLUDED.cost,
    setup_fee = EXCLUDED.setup_fee,
    stock = EXCLUDED.stock,
    updated_at = now();

-- Seed ReliableSite NL Server Inventory Rows
INSERT INTO reliablesite_servers (
    product_id, description, data_center, detail, stock, is_preorder,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month,
    vaultscope_product_id
) VALUES (
    424, 'Pre-Order - AMD Ryzen 9950X 256GB (EU)', 'NL', 'AMD Ryzen 9950X<br />
256 GB DDR5 RAM<br />
2 x 4 TB NVMe (PCIe 4.0)<br />
First Available EU Data Center<br />
Up to 2 Week Setup Time', 100, true,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 599.0000, -1.0000, 3414.3000, 6469.2000, 11500.8000, -1.0000,
    '10000000-0000-4000-8000-000000000424'
) ON CONFLICT (product_id) DO UPDATE SET
    description = EXCLUDED.description,
    data_center = EXCLUDED.data_center,
    detail = EXCLUDED.detail,
    stock = EXCLUDED.stock,
    is_preorder = EXCLUDED.is_preorder,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month,
    vaultscope_product_id = COALESCE(reliablesite_servers.vaultscope_product_id, EXCLUDED.vaultscope_product_id),
    synced_at = now();
INSERT INTO reliablesite_servers (
    product_id, description, data_center, detail, stock, is_preorder,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month,
    vaultscope_product_id
) VALUES (
    425, 'Pre-Order - AMD Epyc 4545P 256GB (EU)', 'NL', 'AMD Epyc 4545P<br />
256 GB DDR5 RAM<br />
4 TB NVMe (PCIe 4.0)<br />
First Available EU Data Center<br />
Up to 2 Week Setup Time', 0, true,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 549.0000, -1.0000, 3129.3000, 5929.2000, 10540.8000, -1.0000,
    '10000000-0000-4000-8000-000000000425'
) ON CONFLICT (product_id) DO UPDATE SET
    description = EXCLUDED.description,
    data_center = EXCLUDED.data_center,
    detail = EXCLUDED.detail,
    stock = EXCLUDED.stock,
    is_preorder = EXCLUDED.is_preorder,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month,
    vaultscope_product_id = COALESCE(reliablesite_servers.vaultscope_product_id, EXCLUDED.vaultscope_product_id),
    synced_at = now();
INSERT INTO reliablesite_servers (
    product_id, description, data_center, detail, stock, is_preorder,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month,
    vaultscope_product_id
) VALUES (
    426, 'Rapid Deploy Server - AMD Ryzen 9950X 256GB (NL)', 'NL', 'AMD Ryzen 9950X<br />
256GB DDR5 RAM<br />
2 x 4 TB NVMe (PCIe 4.0)<br />
Amsterdam, NL Data Center', 5, false,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 599.0000, -1.0000, 3414.3000, 6469.2000, 11500.8000, -1.0000,
    '10000000-0000-4000-8000-000000000426'
) ON CONFLICT (product_id) DO UPDATE SET
    description = EXCLUDED.description,
    data_center = EXCLUDED.data_center,
    detail = EXCLUDED.detail,
    stock = EXCLUDED.stock,
    is_preorder = EXCLUDED.is_preorder,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month,
    vaultscope_product_id = COALESCE(reliablesite_servers.vaultscope_product_id, EXCLUDED.vaultscope_product_id),
    synced_at = now();
INSERT INTO reliablesite_servers (
    product_id, description, data_center, detail, stock, is_preorder,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month,
    vaultscope_product_id
) VALUES (
    427, 'Rapid Deploy Server - AMD Epyc 4545P 256GB (NL)', 'NL', 'AMD Epyc 4545P<br />
256 GB DDR5 RAM<br />
4 TB NVMe<br />
Amsterdam, NL Data Center', 8, false,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 549.0000, -1.0000, 3129.3000, 5929.2000, 10540.8000, -1.0000,
    '10000000-0000-4000-8000-000000000427'
) ON CONFLICT (product_id) DO UPDATE SET
    description = EXCLUDED.description,
    data_center = EXCLUDED.data_center,
    detail = EXCLUDED.detail,
    stock = EXCLUDED.stock,
    is_preorder = EXCLUDED.is_preorder,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month,
    vaultscope_product_id = COALESCE(reliablesite_servers.vaultscope_product_id, EXCLUDED.vaultscope_product_id),
    synced_at = now();
INSERT INTO reliablesite_servers (
    product_id, description, data_center, detail, stock, is_preorder,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month,
    vaultscope_product_id
) VALUES (
    428, 'Rapid Deploy Server - AMD Epyc 4545P 128GB (NL)', 'NL', 'AMD Epyc 4545P<br />
128 GB DDR5 RAM<br />
2 TB NVMe<br />
Amsterdam, NL Data Center', 7, false,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 299.0000, -1.0000, 1704.3000, 3229.2000, 5740.8000, -1.0000,
    '10000000-0000-4000-8000-000000000428'
) ON CONFLICT (product_id) DO UPDATE SET
    description = EXCLUDED.description,
    data_center = EXCLUDED.data_center,
    detail = EXCLUDED.detail,
    stock = EXCLUDED.stock,
    is_preorder = EXCLUDED.is_preorder,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month,
    vaultscope_product_id = COALESCE(reliablesite_servers.vaultscope_product_id, EXCLUDED.vaultscope_product_id),
    synced_at = now();
INSERT INTO reliablesite_servers (
    product_id, description, data_center, detail, stock, is_preorder,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month,
    vaultscope_product_id
) VALUES (
    444, 'Rapid Deploy Server - AMD Epyc 4545P GPU (NL)', 'NL', 'AMD Epyc 4545P<br />
128 GB DDR5 RAM<br />
1TB NVMe<br />
NVIDIA RTX PRO 4500 Blackwell GPU<br />
Amsterdam, Netherlands Data Center', 0, false,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 999.0000, -1.0000, 5694.3000, 10789.2000, 19180.8000, -1.0000,
    '10000000-0000-4000-8000-000000000444'
) ON CONFLICT (product_id) DO UPDATE SET
    description = EXCLUDED.description,
    data_center = EXCLUDED.data_center,
    detail = EXCLUDED.detail,
    stock = EXCLUDED.stock,
    is_preorder = EXCLUDED.is_preorder,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month,
    vaultscope_product_id = COALESCE(reliablesite_servers.vaultscope_product_id, EXCLUDED.vaultscope_product_id),
    synced_at = now();

-- Seed ReliableSite Addon Groups (13 real groups)
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (565, 'Port Speed', 1, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (578, 'Control Panel', 10, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (579, 'Microsoft SQL Server', 11, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (580, 'Total IPs (Port 25 Blocked)', 12, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (618, 'KVM Access', 13, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (642, 'DDoS Protection', 14, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (643, 'Server Management', 15, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (645, 'FTP Backup Space', 16, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (653, 'Graphics Processing Unit', 20, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (654, 'Crypto/Blockchain Activities (Mining, Staking, Validation, etc)?', 21, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (655, 'Operating System', 9, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (659, 'Bandwidth', 2, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
VALUES (661, 'Memory', 3, true)
ON CONFLICT (group_id) DO UPDATE SET description = EXCLUDED.description, sort_order = EXCLUDED.sort_order;

-- Seed Product <-> Addon Group Assignments
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 565, 1)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 659, 2)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 661, 3)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 655, 9)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 578, 10)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 579, 11)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 580, 12)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 618, 13)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 642, 14)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 643, 15)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 645, 16)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 653, 20)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (424, 654, 21)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 565, 1)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 659, 2)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 661, 3)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 655, 9)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 578, 10)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 579, 11)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 580, 12)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 618, 13)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 642, 14)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 643, 15)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 645, 16)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 653, 20)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (425, 654, 21)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 565, 1)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 659, 2)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 661, 3)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 655, 9)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 578, 10)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 579, 11)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 580, 12)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 618, 13)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 642, 14)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 643, 15)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 645, 16)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 653, 20)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (426, 654, 21)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 565, 1)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 659, 2)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 661, 3)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 655, 9)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 578, 10)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 579, 11)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 580, 12)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 618, 13)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 642, 14)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 643, 15)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 645, 16)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 653, 20)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (427, 654, 21)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (428, 565, 1)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (428, 659, 2)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (428, 655, 9)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (428, 578, 10)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (428, 579, 11)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (428, 580, 12)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (428, 618, 13)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (428, 642, 14)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (428, 643, 15)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (428, 645, 16)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (428, 653, 20)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (428, 654, 21)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (444, 565, 1)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (444, 659, 2)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (444, 655, 9)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (444, 578, 10)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (444, 579, 11)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (444, 580, 12)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (444, 618, 13)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (444, 642, 14)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (444, 643, 15)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (444, 645, 16)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;
INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
VALUES (444, 654, 21)
ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order;

-- Seed ReliableSite Addons (all 51 real addons for NL groups)
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    1751, 565, 'Dedicated/Unshared Port', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    1827, 578, 'No Control Panel', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2041, 578, 'cPanel (100 Accounts)', 3,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 66.0000, 198.0000, 376.2000, 712.8000, 1267.2000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2298, 578, 'cPanel (200 Accounts)', 4,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 111.0000, 333.0000, 632.7000, 1198.8000, 2131.2000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2299, 578, 'cPanel (500 Accounts)', 5,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 246.0000, 738.0000, 1402.2000, 2656.8000, 4723.2000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2300, 578, 'cPanel (1000 Accounts)', 6,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 471.0000, 1413.0000, 2684.7000, 5086.8000, 9043.2000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2206, 578, 'Virtualizor (Linux) - License Only', 10,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 9.0000, 27.0000, 51.3000, 97.2000, 172.8000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    1829, 579, 'No SQL Server Database', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    1830, 579, 'SQL Server 2008 R2 Web Edition', 1,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 30.0000, 90.0000, 171.0000, 324.0000, 576.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2335, 579, 'SQL Server 2019 Web Edition', 6,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 30.0000, 90.0000, 171.0000, 324.0000, 576.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2390, 579, 'SQL Server 2022 Web Edition', 7,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 30.0000, 90.0000, 171.0000, 324.0000, 576.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2285, 580, '1 IP Address', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2286, 580, '2 IP Total Addresses', 8,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 2.0000, 6.0000, 11.4000, 21.6000, 38.4000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2287, 580, '5 IP Total Addresses', 9,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 8.0000, 24.0000, 45.6000, 86.4000, 153.6000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2288, 580, '10 IP Total Addresses', 10,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 18.0000, 54.0000, 102.6000, 194.4000, 345.6000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2289, 580, '20 IP Total Addresses', 11,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 38.0000, 114.0000, 216.6000, 410.4000, 729.6000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2293, 580, 'Announce IPs and/or ASN (1-2 Business Days)', 14,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    1965, 618, 'Dedicated KVM Access with Virtual Media', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2156, 642, 'DDoS Protection (NO PROXIES)', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2169, 643, 'Self-Managed/Unmanaged', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2232, 643, 'One Time Migration', 1,
    0, 29.0000, 29.0000, 29.0000, 29.0000, 29.0000, 29.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2171, 643, 'Fully Managed', 2,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 99.0000, 297.0000, 564.3000, 1069.2000, 1900.8000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2194, 645, 'No Backup Space', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2195, 645, '100 GB Backup Space', 1,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 5.0000, 15.0000, 28.5000, 54.0000, 96.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2196, 645, '300 GB Backup Space', 2,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 14.0000, 42.0000, 79.8000, 151.2000, 268.8000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2197, 645, '500 GB Backup Space', 3,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 20.0000, 60.0000, 114.0000, 216.0000, 384.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2198, 645, '750 GB Backup Space', 4,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 30.0000, 90.0000, 171.0000, 324.0000, 576.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2199, 645, '1000 GB Backup Space', 5,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 38.0000, 114.0000, 216.6000, 410.4000, 729.6000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2200, 645, '1500 GB Backup Space', 6,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 50.0000, 150.0000, 285.0000, 540.0000, 960.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2201, 645, '2000 GB Backup Space', 7,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 60.0000, 180.0000, 342.0000, 648.0000, 1152.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2310, 653, 'No GPU', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2309, 653, 'nVidia Quadro RTX 4000', 1,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 99.0000, 0.0000, 564.3000, 1069.2000, 1900.8000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2318, 654, 'No Crypto Mining', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2319, 654, 'Crypto Mining Fee', 1,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 99.0000, 0.0000, 564.3000, 1069.2000, 1900.8000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2336, 655, 'AlmaLinux 9 EFI', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2337, 655, 'CentOS 8 EFI', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2338, 655, 'CloudLinux 9 EFI', 2,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 21.0000, 63.0000, 119.7000, 226.8000, 403.2000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2339, 655, 'Debian 13 EFI', 5,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2350, 655, 'Proxmox-VE 9 EFI', 7,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2346, 655, 'Rocky Linux 9 EFI', 8,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2342, 655, 'Ubuntu 22.04 LTS EFI', 9,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2341, 655, 'Ubuntu 24.04 LTS EFI', 10,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2345, 655, 'Windows Server 2022 Standard EFI', 18,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 40.0000, 120.0000, 228.0000, 432.0000, 768.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2357, 655, 'Windows Server 2022 Standard EFI Trial', 19,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2391, 659, '1 Gbps Unmetered', 11,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2392, 659, '2 Gbps Unmetered', 12,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 99.0000, 297.0000, 564.3000, 1069.2000, 1900.8000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2393, 659, '3 Gbps Unmetered', 13,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 190.0000, 570.0000, 1083.0000, 2052.0000, 3648.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2394, 659, '5 Gbps Unmetered', 14,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 350.0000, 1050.0000, 1995.0000, 3780.0000, 6720.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2395, 659, '10 Gbps Unmetered', 15,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 599.0000, 1797.0000, 3414.3000, 6469.2000, 11500.8000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2421, 661, '256 DDR5 Non-ECC RAM', 0,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;
INSERT INTO reliablesite_addons (
    addon_id, group_id, addon_description, sort_order,
    setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
    recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
) VALUES (
    2422, 661, '256 GB DDR5 ECC RAM (Up to 1 week setup)', 1,
    0, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0, 340.0000, 1020.0000, 1938.0000, 3672.0000, 6528.0000, 0.0000
) ON CONFLICT (addon_id) DO UPDATE SET
    group_id = EXCLUDED.group_id,
    addon_description = EXCLUDED.addon_description,
    sort_order = EXCLUDED.sort_order,
    setup_1_hour = EXCLUDED.setup_1_hour,
    setup_1_month = EXCLUDED.setup_1_month,
    setup_3_month = EXCLUDED.setup_3_month,
    setup_6_month = EXCLUDED.setup_6_month,
    setup_12_month = EXCLUDED.setup_12_month,
    setup_24_month = EXCLUDED.setup_24_month,
    setup_36_month = EXCLUDED.setup_36_month,
    recurring_1_hour = EXCLUDED.recurring_1_hour,
    recurring_1_month = EXCLUDED.recurring_1_month,
    recurring_3_month = EXCLUDED.recurring_3_month,
    recurring_6_month = EXCLUDED.recurring_6_month,
    recurring_12_month = EXCLUDED.recurring_12_month,
    recurring_24_month = EXCLUDED.recurring_24_month,
    recurring_36_month = EXCLUDED.recurring_36_month;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'vaultscope') THEN
        GRANT ALL PRIVILEGES ON TABLE
            reliablesite_servers,
            reliablesite_addon_groups,
            reliablesite_product_addon_groups,
            reliablesite_addons
        TO vaultscope;
    END IF;
END $$;
