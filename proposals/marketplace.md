# Proposal: software marketplace (for docs/, written by the human)

**What it is.** A customer opens *Marketplace > Server software* and installs, updates or removes
server software in the groups they know from Webuzo: databases, web servers, Java tools and
containers, message queues, version control, security, utilities, modules, stacks, statistics,
libraries. API: `GET /api/v1/client/marketplace`, `POST .../{slug}/install`, `POST .../{slug}/update`,
`DELETE .../{slug}`.

**How.** One file per software in `deploy/marketplace/` (compiled into the control plane). Each is
`container` (run from an open-source image pinned by digest, in the same hardened way as every
client container), `builtin` (the panel already has it, with where) or `unavailable` (with the
reason). All 241 Webuzo names are covered by exactly one entry (`covers`). Closed-source items
(MSSQL, ObjectDB) are not listed.

**Keeping it current.** `deploy/marketplace/refresh.py --check` lists newer releases;
`--write` pins them; `verify.sh` runs every image in the sandbox on the build box. A customer then sees
"update available" and updates in one click (the container is run again from the new image on the
same `/data` folder; what a software keeps elsewhere in its own file system starts fresh).

**Not installable yet (needs a decision).** MongoDB, RabbitMQ, Cassandra, Neo4j, ClamAV and ZooKeeper
start as root and then switch user, and Elasticsearch and Valkey need a data folder their own user owns; our containers have no capabilities and `/data` belongs to root. Running them needs
a narrowly limited "run as the image's own user" setting for administrator-approved images.
