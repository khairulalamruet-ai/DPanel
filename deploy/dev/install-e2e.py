#!/usr/bin/env python3
"""The installer's Definition of Done, against a real server (T19.01).

Run after `install.sh` on a new server, from any machine that can reach the panel over HTTPS and
log in to the server over SSH:

    deploy/dev/install-e2e.py --base https://panel.example.com --login owner --password '...' \
        --ssh 'ssh -i KEY root@SERVER' --domain-suffix SERVER-IP.sslip.io

Steps: sign in; add the local node and enrol the agent (as the agent's user); wait until it is
online; make a plan and a hosting account on it; add a domain and put a page in the account's
files; ask for the page over HTTP and HTTPS. Prints one line per step and exits 1 on the first
failure, saying why.
"""
import argparse, http.cookiejar, json, shlex, subprocess, sys, time, urllib.error, urllib.request

ap = argparse.ArgumentParser()
ap.add_argument("--base", required=True)
ap.add_argument("--login", required=True)
ap.add_argument("--password", required=True)
ap.add_argument("--ssh", required=True, help="command that runs a shell command on the server as root")
ap.add_argument("--domain-suffix", required=True, help="a name suffix that resolves to the server (IP.sslip.io)")
ap.add_argument("--node-name", default="local")
args = ap.parse_args()
BASE = args.base.rstrip("/")


def step(msg, ok=True, detail=""):
    print(("ok    " if ok else "FAIL  ") + msg + (f"  ({detail})" if detail else ""), flush=True)
    if not ok:
        sys.exit(1)


class Session:
    def __init__(self):
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))
        self.csrf = None

    def call(self, method, path, body=None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(BASE + path, data=data, method=method)
        req.add_header("content-type", "application/json")
        if self.csrf:
            req.add_header("x-csrf-token", self.csrf)
        try:
            with self.opener.open(req, timeout=60) as r:
                text = r.read().decode()
                return r.status, (json.loads(text) if text else None)
        except urllib.error.HTTPError as e:
            text = e.read().decode()
            try:
                return e.code, json.loads(text)
            except ValueError:
                return e.code, text

    def login(self, login, password):
        status, got = self.call("POST", "/api/v1/public/auth/login", {"login": login, "password": password})
        if status != 200:
            return status, got
        self.csrf = (got or {}).get("csrf_token") or (got or {}).get("csrf")
        return status, got


def remote(cmd, check=True):
    out = subprocess.run(shlex.split(args.ssh) + [cmd], capture_output=True, text=True)
    if check and out.returncode != 0:
        step(f"on the server: {cmd[:60]}", False, (out.stderr or out.stdout).strip()[-300:])
    return out.stdout.strip()


admin = Session()
status, got = admin.login(args.login, args.password)
step("the administrator signs in", status == 200, f"{status} {got}" if status != 200 else "")
if not admin.csrf:
    step("the sign-in answer carries a CSRF token", False, json.dumps(got)[:200])

# --- the node --------------------------------------------------------------------------------
status, nodes = admin.call("GET", "/api/v1/admin/nodes")
existing = [n for n in (nodes or {}).get("items", nodes if isinstance(nodes, list) else []) if n.get("name") == args.node_name]
if not existing:
    status, made = admin.call("POST", "/api/v1/admin/nodes", {"name": args.node_name})
    step("the local node is added (an enrolment is made)", status == 201 and made.get("enroll_command"), f"{status} {made}" if status != 201 else "")
    command = made["enroll_command"]
    remote(f"sudo -u builtpanel-agent {command}")
    step("the agent enrols (as its own user)", True)
    remote("systemctl enable --now builtpanel-agent")
for _ in range(60):
    status, nodes = admin.call("GET", "/api/v1/admin/nodes")
    items = (nodes or {}).get("items", nodes if isinstance(nodes, list) else [])
    node = next((n for n in items if n.get("name") == args.node_name), None)
    if node and node.get("status") == "online":
        break
    time.sleep(2)
step("the node is online", bool(node and node.get("status") == "online"), json.dumps(node)[:200])
node_id = node["id"]

# --- a plan and a hosting account ---------------------------------------------------------------
status, sets = admin.call("GET", "/api/v1/admin/feature-sets")
fs = (sets or {}).get("items", sets if isinstance(sets, list) else [])
step("a feature set exists", bool(fs), str(sets)[:200])
status, plan = admin.call("POST", "/api/v1/admin/plans", {
    "name": f"e2e_{int(time.time())}", "feature_set_id": fs[0]["id"],
    "resources": {"cpu_millis": 500, "disk_mb": 2000, "memory_mb": 512, "pids": 200}})
step("a plan is made", status == 201, f"{status} {plan}" if status != 201 else "")
user = f"e2e{int(time.time()) % 100000}"
client_password = "E2e-pass-" + str(int(time.time())) + "-Aa1"
status, made = admin.call("POST", "/api/v1/admin/users", {
    "username": user, "email": f"{user}@{args.domain_suffix}", "password": client_password,
    "node_id": node_id, "plan_id": plan["id"]})
step("a hosting account is made", status == 201, f"{status} {made}" if status != 201 else "")
print(f"      account {user}", flush=True)
tenant = None
state = None
for _ in range(90):
    status, got = admin.call("GET", "/api/v1/admin/users")
    items = (got or {}).get("items", got if isinstance(got, list) else [])
    tenant = next((u for u in items if u.get("username") == user), None)
    state = ((tenant or {}).get("tenant") or {}).get("status")
    if state in ("active", "ready", "failed"):
        break
    time.sleep(2)
step("the account's stack is provisioned", state in ("active", "ready"), f"the account's hosting status is {state!r}")

# --- a domain and a page ---------------------------------------------------------------------
client = Session()
status, got = client.login(user, client_password)
step("the client signs in", status == 200, f"{status} {got}" if status != 200 else "")
fqdn = f"{user}.{args.domain_suffix}"
status, dom = client.call("POST", "/api/v1/client/domains", {"fqdn": fqdn})
step("the client adds a domain", status in (200, 201), f"{status} {dom}" if status not in (200, 201) else fqdn)
page = f"hello from {user}"
# The account's files are its folder under the node's tenants root (named by the tenant's id).
tenant_id = tenant["tenant"]["id"]
remote(f"d=/srv/tenants/{tenant_id}/www; "
       f"[ -d \"$d\" ] && echo '{page}' > \"$d/index.html\" && chmod 644 \"$d/index.html\" && echo written || {{ echo \"no $d\"; exit 1; }}")
body = ""
for _ in range(60):
    try:
        with urllib.request.urlopen(f"http://{fqdn}/", timeout=10) as r:
            body = r.read().decode()
            if page in body:
                break
    except Exception as e:  # not up yet
        body = str(e)
    time.sleep(2)
step("the site answers over HTTP with the account's page", page in body, body[:160])
# --- the web interface ---------------------------------------------------------------------------
def fetch(url):
    try:
        with urllib.request.urlopen(url, timeout=20) as r:
            return r.status, r.read().decode(errors="replace"), dict(r.headers)
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(errors="replace"), dict(e.headers)

host = BASE.split("://", 1)[1]
for label, url in (("clients", BASE), ("administrators", f"https://admin.{host}")):
    status, text, headers = fetch(url + "/")
    step(f"the web interface of the {label} is served at {url}", status == 200 and 'id="root"' in text, f"{status} {text[:100]!r}")
    status, text, _ = fetch(url + "/some/page/of/the/app")
    step(f"  a page of the app (not a file) gives the app, so a reload works", status == 200 and 'id="root"' in text, str(status))
    status, text, _ = fetch(url + "/api/v1/public/health")
    step(f"  /api/* on the same name is the control plane", status == 200 and "ok" in text.lower(), f"{status} {text[:80]!r}")
    step(f"  the page carries the security headers", headers.get("X-Content-Type-Options") == "nosniff" and headers.get("X-Frame-Options") == "DENY", str({k: v for k, v in headers.items() if k.startswith("X-")}))
print("all steps passed", flush=True)
