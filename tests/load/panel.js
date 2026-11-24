// Load test of the control plane (T19.05): 100 people on the panel at once and 1000
// operations a minute, against a database holding 500 tenants.
//
//   k6 run -e BASE=http://127.0.0.1:18080 -e ACCOUNTS=500 -e PASSWORD=... tests/load/panel.js
//
// The accounts load0..load<ACCOUNTS-1> are made by deploy/dev/load-live.sh. Pass criteria
// (thresholds): 95% of API answers and of operation requests under 300 ms, no failed request,
// every operation request answered 200 by the (simulated) node, and the 1000 a minute really sent.
import http from 'k6/http';
import { check, sleep } from 'k6';
import { Counter } from 'k6/metrics';

const BASE = __ENV.BASE || 'http://127.0.0.1:18080';
const ACCOUNTS = Number(__ENV.ACCOUNTS || 500);
const PASSWORD = __ENV.PASSWORD;
const DURATION = __ENV.DURATION || '2m';
const queued = new Counter('operations_queued');

export const options = {
  scenarios: {
    people: {
      executor: 'constant-vus',
      vus: 100,
      duration: DURATION,
      exec: 'person',
    },
    operations: {
      executor: 'constant-arrival-rate',
      rate: 1000,
      timeUnit: '1m',
      duration: DURATION,
      preAllocatedVUs: 20,
      maxVUs: 50,
      exec: 'operation',
    },
  },
  thresholds: {
    // Only the panel's own answers; the sign-in is measured on its own.
    'http_req_duration{kind:api}': ['p(95)<300'],
    'http_req_failed{kind:api}': ['rate==0'],
    'http_req_duration{kind:operation}': ['p(95)<300'],
    'http_req_failed{kind:operation}': ['rate==0'],
    'http_req_failed{kind:login}': ['rate==0'],
    // The arrival rate was reached: no operation request was left unsent.
    dropped_iterations: ['count==0'],
    checks: ['rate==1'],
  },
};

const READS = [
  '/api/v1/auth/me',
  '/api/v1/client/apps',
  '/api/v1/client/backups',
  '/api/v1/client/cache',
  '/api/v1/client/cron-jobs',
  '/api/v1/client/databases',
  '/api/v1/client/db-users',
  '/api/v1/client/domains',
  '/api/v1/client/ftp-accounts',
  '/api/v1/client/malware-scans',
  '/api/v1/client/usage',
];

// The first few failures say what happened (status and the start of the body).
let shown = 0;
function why(what, res) {
  if (shown++ < 3) console.warn(`${what}: ${res.status} ${String(res.body).slice(0, 160)}`);
}

// Everybody signs in once before the clock starts (the sign-in's argon2 cost is measured by
// its own tag and is not part of "API latency"): 100 people and 50 accounts for the operations.
function signIn(n) {
  const res = http.post(
    `${BASE}/api/v1/public/auth/login`,
    JSON.stringify({ login: `load${n}`, password: PASSWORD }),
    // A fresh cookie jar for every sign-in: the panel ends any session presented at login (session
    // fixation defence), and a shared jar would present the previous person's.
    { headers: { 'content-type': 'application/json' }, tags: { kind: 'login' }, jar: new http.CookieJar() },
  );
  if (!check(res, { 'signed in': (r) => r.status === 200 })) why('login', res);
  // The session cookie is Secure, which a plain-HTTP load target would not get back by itself.
  const cookie = res.headers['Set-Cookie'] || '';
  return {
    cookie: cookie.split(';')[0],
    csrf: res.status === 200 ? res.json('csrf_token') : '',
    n,
  };
}

export function setup() {
  const sessions = [];
  for (let n = 0; n < 150; n++) sessions.push(signIn(n % ACCOUNTS));
  // = sessions.map((x, i) => http.get(`${BASE}/api/v1/auth/me`, { headers: { cookie: x.cookie } }).status === 200 ? i : -1).filter((i) => i >= 0);
  return sessions;
}

export function person(sessions) {
  const session = sessions[(__VU - 1) % 100];
  const path = READS[Math.floor(Math.random() * READS.length)];
  const res = http.get(`${BASE}${path}`, {
    headers: { cookie: session.cookie },
    tags: { kind: 'api' },
  });
  if (!check(res, { [`GET ${path} is 200`]: (r) => r.status === 200 })) why(`GET ${path}`, res);
  sleep(0.5 + Math.random());
}

export function operation(sessions) {
  const session = sessions[100 + (__VU % 50)];
  const res = http.post(
    `${BASE}/api/v1/client/containers/bp-load${session.n}-web/restart`,
    '{}',
    {
      headers: {
        cookie: session.cookie,
        'x-csrf-token': session.csrf,
        'content-type': 'application/json',
      },
      tags: { kind: 'operation' },
    },
  );
  const ok = check(res, { 'operation done (200)': (r) => r.status === 200 });
  if (!ok) why('operation', res);
  if (ok) queued.add(1);
}
