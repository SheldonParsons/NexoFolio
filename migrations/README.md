# Database migrations

`202609140001_access.sql` creates the access-owned users, internal sessions,
projects, user/project access and login audit tables, plus a sync generation sequence.
The SQLx runner maintains `_sqlx_migrations`.

Run `nexofolio-admin migrate` explicitly before serving business requests.
API and worker never migrate automatically. Applied migration files are immutable;
future changes require a new migration and tests against existing data.
