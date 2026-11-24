# ADR 0003 — PostgreSQL for the panel's own data

Status: accepted (implemented since phase 00)

## Context
The panel stores accounts, the operations ledger, the event outbox, audit records, usage samples
and settings. It needs transactions that span several tables (an account, its hosting account and
the work that sets it up are written together), row locking for queues (`FOR UPDATE SKIP LOCKED`),
partitioning for usage data, and JSON columns for module settings.

## Decision
Use **PostgreSQL** (current major) for all panel data, with SQLx migrations in `migrations/`
(forward `up` and a `down` for each). Redis is used only for sessions and short-lived counters;
losing it logs people out and nothing else. The tenants' own databases are separate servers
(MariaDB per node; PostgreSQL for tenants comes later) and never share a server with the panel.

## Alternatives considered
- **MariaDB/MySQL**: workable, but weaker transactional DDL, no `SKIP LOCKED` queue semantics until
  recently, and we want the panel's database to differ from the tenants'.
- **SQLite**: no network access for the worker processes and the license authority.

## Consequences
- The operations ledger is a table, not a broker: one fewer moving part to install.
- Upgrades run migrations before the new binaries start (`bpctl migrate`), and a migration that
  fails leaves the old version running.
