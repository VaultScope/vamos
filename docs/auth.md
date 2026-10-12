# API Authentication & RBAC

## Mechanism
- The API is the issuer of JWTs, returning them in `auth.rs`.
- Two audiences exist: `Audience::Storefront` and `Audience::Admin`.
- Validations are performed inside Axum extractors: `AuthCustomer` and `AuthAdmin`.

## RBAC (Role-Based Access Control)
- Located in `src/middleware/rbac.rs`.
- Validates that the Admin token corresponds to a `Staff` record.
- Retrieves the associated `Role` and checks its `permissions` array for the requested scope (or a `*` wildcard).
- Applied via the `require_permission!(auth, state, "permission.name")` macro in route handlers.

## Security Flaws
- The `dev_login` endpoint completely bypasses OIDC without environment checks.
- Deprecated functions like `verify_legacy` remain in code and must be removed to fix build warnings.
