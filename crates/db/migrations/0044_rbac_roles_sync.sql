-- 0044_rbac_roles_sync.sql
-- Synchronize and expand default CAMOS RBAC role templates

UPDATE roles
SET permissions = ARRAY['*'],
    updated_at = now()
WHERE name IN ('Owner', 'Super Admin');

INSERT INTO roles (id, name, permissions, mapped_group) VALUES
    (
        '0ca7a95e-5bf1-4519-b3f6-0e207ff76a3c',
        'Support',
        ARRAY[
            'customers.view',
            'customers.impersonate',
            'tickets.view',
            'tickets.create',
            'tickets.reply',
            'tickets.manage',
            'services.view',
            'invoices.view',
            'notifications.view'
        ],
        'vs-support'
    ),
    (
        'aa1efaad-b5bd-4548-a6ec-41b92fbac2b2',
        'Billing',
        ARRAY[
            'customers.view',
            'customers.edit',
            'billing.view',
            'billing.manage',
            'invoices.view',
            'invoices.create',
            'invoices.edit',
            'invoices.delete',
            'invoices.manage',
            'tax.manage',
            'coupons.manage',
            'reports.view',
            'notifications.view'
        ],
        'vs-billing'
    ),
    (
        '332da0e5-a529-4666-b247-6d35d3f71b4a',
        'Technical',
        ARRAY[
            'customers.view',
            'tickets.view',
            'tickets.reply',
            'tickets.manage',
            'services.view',
            'services.power',
            'services.reinstall',
            'services.suspend',
            'services.manage',
            'jobs.view',
            'jobs.manage',
            'notifications.view'
        ],
        'vs-technical'
    ),
    (
        '443eb1f6-b63a-4777-a358-7e46e4a82c5b',
        'Infrastructure Admin',
        ARRAY[
            'services.view',
            'services.manage',
            'services.power',
            'services.reinstall',
            'services.suspend',
            'services.terminate',
            'jobs.view',
            'jobs.manage',
            'products.view',
            'products.manage',
            'products.create',
            'products.edit',
            'products.delete',
            'forms.manage',
            'internal_tools.view',
            'notifications.view'
        ],
        'vs-infra-admin'
    ),
    (
        '554fc2a7-c74b-4888-b469-8f57f5b93d6c',
        'Account Manager',
        ARRAY[
            'customers.view',
            'customers.manage',
            'customers.create',
            'customers.edit',
            'customers.impersonate',
            'customers.delete',
            'staff.view',
            'staff.manage',
            'staff.delete',
            'accounts.delete',
            'invoices.view',
            'services.view',
            'tickets.view',
            'activity.view',
            'notifications.view'
        ],
        'vs-account-manager'
    )
ON CONFLICT (name) DO UPDATE
SET permissions = EXCLUDED.permissions,
    updated_at = now();
