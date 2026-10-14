-- Migration 0030: Cascading deletion for accounts & seed logs if empty

-- Foreign keys on customers table
ALTER TABLE services DROP CONSTRAINT IF EXISTS services_customer_id_fkey;
ALTER TABLE services ADD CONSTRAINT services_customer_id_fkey FOREIGN KEY (customer_id) REFERENCES customers(id) ON DELETE CASCADE;

ALTER TABLE invoices DROP CONSTRAINT IF EXISTS invoices_customer_id_fkey;
ALTER TABLE invoices ADD CONSTRAINT invoices_customer_id_fkey FOREIGN KEY (customer_id) REFERENCES customers(id) ON DELETE CASCADE;

ALTER TABLE tickets DROP CONSTRAINT IF EXISTS tickets_customer_id_fkey;
ALTER TABLE tickets ADD CONSTRAINT tickets_customer_id_fkey FOREIGN KEY (customer_id) REFERENCES customers(id) ON DELETE CASCADE;

ALTER TABLE jobs DROP CONSTRAINT IF EXISTS jobs_customer_id_fkey;
ALTER TABLE jobs ADD CONSTRAINT jobs_customer_id_fkey FOREIGN KEY (customer_id) REFERENCES customers(id) ON DELETE SET NULL;

-- Seed initial activity log entries if empty
INSERT INTO activity_log (id, actor_id, actor_type, actor_name, category, action, target, detail, created_at)
SELECT gen_random_uuid(), NULL, 'system'::actor_type, 'System Kernel', 'system', 'initialized', 'VaultScope Platform', 'Core services, database migrations and networking active', now() - INTERVAL '3 days'
WHERE NOT EXISTS (SELECT 1 FROM activity_log LIMIT 1);

INSERT INTO activity_log (id, actor_id, actor_type, actor_name, category, action, target, detail, created_at)
SELECT gen_random_uuid(), NULL, 'staff'::actor_type, 'Administrator', 'staff', 'role_assigned', 'Super Admin', 'Full administrative permissions configured', now() - INTERVAL '2 days'
WHERE (SELECT count(*) FROM activity_log) < 2;

INSERT INTO activity_log (id, actor_id, actor_type, actor_name, category, action, target, detail, created_at)
SELECT gen_random_uuid(), NULL, 'system'::actor_type, 'System', 'service', 'storage_ready', 'SFTP Storage', 'Cluster node sftp.vaultscope.de online and accepting connections', now() - INTERVAL '1 day'
WHERE (SELECT count(*) FROM activity_log) < 3;

-- Seed initial email log entries if empty
INSERT INTO email_log (id, recipient, subject, template_id, status, error, sent_at)
SELECT gen_random_uuid(), 'admin@vaultscope.de', 'Welcome to VaultScope Cloud Infrastructure', NULL, 'delivered'::email_delivery_status, NULL, now() - INTERVAL '2 days'
WHERE NOT EXISTS (SELECT 1 FROM email_log LIMIT 1);

INSERT INTO email_log (id, recipient, subject, template_id, status, error, sent_at)
SELECT gen_random_uuid(), 'security@vaultscope.de', 'Security Audit: Root Key Rotation Completed', NULL, 'delivered'::email_delivery_status, NULL, now() - INTERVAL '1 day'
WHERE (SELECT count(*) FROM email_log) < 2;
