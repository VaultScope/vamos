INSERT INTO email_templates (name, category, subject, body, variables) VALUES
('Sign-Up Confirmation', 'account', 'Welcome to VaultScope', '<p>Thank you for signing up for VaultScope!</p>', '["name", "login_url"]'),
('Email Verification', 'account', 'Verify Your Email Address', '<p>Please click the link below to verify your email address:</p><p><a href="{{verification_url}}">Verify Email</a></p>', '["name", "verification_url"]'),
('Invoice Paid', 'billing', 'Invoice Paid - Thank You', '<p>Your invoice #{{invoice_id}} has been paid successfully.</p>', '["name", "invoice_id", "amount", "currency"]'),
('Invoice Payment Failed', 'billing', 'Payment Failed for Invoice #{{invoice_id}}', '<p>We were unable to process your payment for invoice #{{invoice_id}}.</p>', '["name", "invoice_id", "amount", "currency", "payment_url"]'),
('Invoice Payment awaiting', 'billing', 'Invoice #{{invoice_id}} is Awaiting Payment', '<p>Your invoice #{{invoice_id}} is now awaiting payment.</p>', '["name", "invoice_id", "amount", "currency", "due_date", "payment_url"]'),
('Invoice Payment Warning', 'billing', 'Warning: Invoice #{{invoice_id}} is Overdue', '<p>Your invoice #{{invoice_id}} is overdue. Please pay it as soon as possible.</p>', '["name", "invoice_id", "amount", "currency", "due_date", "payment_url"]'),
('Invoice Final Notice', 'billing', 'Final Notice: Invoice #{{invoice_id}}', '<p>This is the final notice for your overdue invoice #{{invoice_id}}. Your services may be suspended.</p>', '["name", "invoice_id", "amount", "currency", "due_date", "payment_url"]')
ON CONFLICT (name) DO NOTHING;
