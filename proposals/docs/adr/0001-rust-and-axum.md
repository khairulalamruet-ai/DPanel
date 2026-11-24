# ADR 0001 — Rust and Axum for the backend

Status: accepted (implemented since phase 00)

## Context
The control plane, the node agent, the privileged helper, the license authority and the CLI
(`bpctl`) are long-running programs that handle other people's sites, credentials and root-adjacent
work. They need memory safety, a small footprint on every node, one toolchain for all of them, and
types that carry through from the database to the wire.

## Decision
Write the backend in **Rust** (edition 2024, `unsafe` forbidden in our crates, clippy with
`-D warnings`) on **Axum** with Tokio. SQL goes through **SQLx** with compile-time checked queries
(offline cache in `.sqlx/`). The API's description (`openapi.json`) is generated from the code with
utoipa, so the web app and the code cannot drift apart.

## Alternatives considered
- **Go**: simple deployment and good libraries, but no sum types, a weaker story for "this value
  was checked" (our domain types), and nil/zero-value mistakes in security code.
- **Node.js/TypeScript**: one language with the web app, but a large dependency surface, and a
  runtime we would not want on every node next to the tenants' own Node.js apps.
- **Python**: fastest to write; slowest, and the least safe for a privileged agent.

## Consequences
- One static binary per program; no interpreter on the nodes.
- Compile times are long: a remote build box (`deploy/dev/remote.sh`) does the heavy checks.
- Fewer people know Rust; the code keeps to plain idioms, and every module follows the same shape
  (`crates/modules/<name>`, a manifest, routes, live tests).
