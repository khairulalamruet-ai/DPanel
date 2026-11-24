import { PageTemplateItem } from "./types";

export const defaultSuspendedDomainHtml = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Domain Suspended - {{domain}}</title>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background-color: #0f172a;
      color: #f8fafc;
      min-height: 100vh;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 1.5rem;
    }
    .card {
      background: #1e293b;
      border: 1px solid #334155;
      border-radius: 1rem;
      max-width: 540px;
      width: 100%;
      padding: 2.5rem;
      text-align: center;
      box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.5);
    }
    .badge {
      display: inline-block;
      padding: 0.35rem 0.85rem;
      border-radius: 9999px;
      font-size: 0.85rem;
      font-weight: 600;
      background: #7f1d1d;
      color: #fecaca;
      margin-bottom: 1.25rem;
    }
    h1 {
      font-size: 1.75rem;
      font-weight: 700;
      color: #f1f5f9;
      margin-bottom: 0.75rem;
    }
    p {
      color: #94a3b8;
      font-size: 0.95rem;
      line-height: 1.6;
      margin-bottom: 1.5rem;
    }
    .domain-box {
      background: #0f172a;
      border: 1px solid #334155;
      border-radius: 0.5rem;
      padding: 0.85rem;
      font-family: monospace;
      color: #38bdf8;
      font-size: 1rem;
      margin-bottom: 1.5rem;
      word-break: break-all;
    }
    .reason-box {
      background: rgba(239, 68, 68, 0.1);
      border-left: 3px solid #ef4444;
      padding: 0.75rem 1rem;
      text-align: left;
      border-radius: 0.25rem;
      color: #fca5a5;
      font-size: 0.875rem;
      margin-bottom: 1.75rem;
    }
    .btn {
      display: inline-block;
      background: #2563eb;
      color: #ffffff;
      text-decoration: none;
      font-weight: 600;
      padding: 0.75rem 1.5rem;
      border-radius: 0.5rem;
      transition: background 0.2s;
    }
    .btn:hover { background: #1d4ed8; }
    .footer {
      margin-top: 2rem;
      font-size: 0.75rem;
      color: #64748b;
    }
  </style>
</head>
<body>
  <div class="card">
    <div class="badge">Notice</div>
    <h1>Domain Suspended</h1>
    <p>This domain name has been suspended and its services are temporarily disabled.</p>
    <div class="domain-box">{{domain}}</div>
    <div class="reason-box"><strong>Reason:</strong> {{reason}}</div>
    <a href="mailto:{{contact_email}}" class="btn">Contact Administrator</a>
    <div class="footer">&copy; {{year}} {{panel_name}}. All rights reserved.</div>
  </div>
</body>
</html>`;

export const defaultSuspendedUserHtml = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Account Suspended</title>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background-color: #0f172a;
      color: #f8fafc;
      min-height: 100vh;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 1.5rem;
    }
    .card {
      background: #1e293b;
      border: 1px solid #334155;
      border-radius: 1rem;
      max-width: 540px;
      width: 100%;
      padding: 2.5rem;
      text-align: center;
      box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.5);
    }
    .icon {
      width: 56px;
      height: 56px;
      border-radius: 50%;
      background: rgba(245, 158, 11, 0.15);
      color: #f59e0b;
      display: flex;
      align-items: center;
      justify-content: center;
      margin: 0 auto 1.25rem;
      font-size: 1.75rem;
      font-weight: bold;
    }
    h1 {
      font-size: 1.75rem;
      font-weight: 700;
      color: #f1f5f9;
      margin-bottom: 0.75rem;
    }
    p {
      color: #94a3b8;
      font-size: 0.95rem;
      line-height: 1.6;
      margin-bottom: 1.5rem;
    }
    .notice {
      background: rgba(245, 158, 11, 0.1);
      border-left: 3px solid #f59e0b;
      padding: 0.75rem 1rem;
      text-align: left;
      border-radius: 0.25rem;
      color: #fcd34d;
      font-size: 0.875rem;
      margin-bottom: 1.75rem;
    }
    .btn {
      display: inline-block;
      background: #2563eb;
      color: #ffffff;
      text-decoration: none;
      font-weight: 600;
      padding: 0.75rem 1.5rem;
      border-radius: 0.5rem;
      transition: background 0.2s;
    }
    .btn:hover { background: #1d4ed8; }
    .footer {
      margin-top: 2rem;
      font-size: 0.75rem;
      color: #64748b;
    }
  </style>
</head>
<body>
  <div class="card">
    <div class="icon">!</div>
    <h1>Account Suspended</h1>
    <p>The hosting account associated with this site is currently suspended.</p>
    <div class="notice">
      If you are the website owner, please check your registered email or contact support to resolve pending billing or administrative requirements.
    </div>
    <a href="mailto:{{contact_email}}" class="btn">Contact Support</a>
    <div class="footer">&copy; {{year}} {{panel_name}}. All rights reserved.</div>
  </div>
</body>
</html>`;

export const defaultError404Html = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>404 - Page Not Found</title>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background-color: #0b0f19;
      color: #f3f4f6;
      min-height: 100vh;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 1.5rem;
    }
    .container {
      text-align: center;
      max-width: 500px;
      width: 100%;
    }
    .code {
      font-size: 6rem;
      font-weight: 900;
      letter-spacing: -0.05em;
      line-height: 1;
      background: linear-gradient(135deg, #38bdf8, #818cf8);
      -webkit-background-clip: text;
      -webkit-text-fill-color: transparent;
      margin-bottom: 0.5rem;
    }
    h1 {
      font-size: 1.75rem;
      font-weight: 700;
      margin-bottom: 0.75rem;
      color: #f9fafb;
    }
    p {
      color: #9ca3af;
      font-size: 1rem;
      line-height: 1.6;
      margin-bottom: 1.75rem;
    }
    .path-box {
      display: inline-block;
      background: #1f2937;
      border: 1px solid #374151;
      border-radius: 0.375rem;
      padding: 0.35rem 0.75rem;
      font-family: monospace;
      color: #e5e7eb;
      font-size: 0.875rem;
      margin-bottom: 1.5rem;
    }
    .btn {
      display: inline-block;
      background: #3b82f6;
      color: #ffffff;
      text-decoration: none;
      font-weight: 500;
      padding: 0.65rem 1.25rem;
      border-radius: 0.375rem;
    }
    .btn:hover { background: #2563eb; }
    .footer {
      margin-top: 2.5rem;
      font-size: 0.75rem;
      color: #4b5563;
    }
  </style>
</head>
<body>
  <div class="container">
    <div class="code">404</div>
    <h1>Page Not Found</h1>
    <p>The page or resource you requested could not be located on this server.</p>
    <div class="path-box">{{path}}</div>
    <div>
      <a href="/" class="btn">Return to Homepage</a>
    </div>
    <div class="footer">&copy; {{year}} {{panel_name}} Web Services</div>
  </div>
</body>
</html>`;

export const defaultError502Html = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>502 - Bad Gateway</title>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background-color: #0b0f19;
      color: #f3f4f6;
      min-height: 100vh;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 1.5rem;
    }
    .card {
      background: #111827;
      border: 1px solid #1f2937;
      border-radius: 0.75rem;
      max-width: 580px;
      width: 100%;
      padding: 2.5rem;
      text-align: center;
      box-shadow: 0 20px 25px -5px rgba(0, 0, 0, 0.4);
    }
    .code {
      font-size: 4rem;
      font-weight: 800;
      color: #f87171;
      line-height: 1;
      margin-bottom: 0.5rem;
    }
    h1 {
      font-size: 1.5rem;
      font-weight: 700;
      color: #f9fafb;
      margin-bottom: 0.75rem;
    }
    p {
      color: #9ca3af;
      font-size: 0.95rem;
      line-height: 1.6;
      margin-bottom: 1.5rem;
    }
    .status-flow {
      display: flex;
      justify-content: center;
      align-items: center;
      gap: 0.5rem;
      background: #1f2937;
      padding: 0.75rem;
      border-radius: 0.5rem;
      font-size: 0.85rem;
      margin-bottom: 1.5rem;
    }
    .node-ok { color: #34d399; font-weight: 600; }
    .node-err { color: #f87171; font-weight: 600; }
    .arrow { color: #6b7280; }
    .footer {
      font-size: 0.75rem;
      color: #6b7280;
    }
  </style>
</head>
<body>
  <div class="card">
    <div class="code">502</div>
    <h1>Bad Gateway</h1>
    <p>The web server received an invalid response while acting as a gateway or proxy to the upstream PHP application ({{service}}).</p>
    <div class="status-flow">
      <span class="node-ok">Client</span>
      <span class="arrow">&rarr;</span>
      <span class="node-ok">Reverse Proxy</span>
      <span class="arrow">&rarr;</span>
      <span class="node-err">Upstream PHP (Unreachable)</span>
    </div>
    <div class="footer">&copy; {{year}} {{panel_name}} Web Services</div>
  </div>
</body>
</html>`;

export const initialPageTemplates: PageTemplateItem[] = [
  {
    id: "tpl_suspended_domain",
    type: "suspended_domain",
    title: "Domain Suspended",
    description: "Served when a specific domain is suspended by the administrator.",
    statusCode: 451,
    content: defaultSuspendedDomainHtml,
    defaultContent: defaultSuspendedDomainHtml,
    updatedAt: "2026-11-10T12:00:00Z",
    isCustomized: false,
    variables: [
      { name: "{{domain}}", description: "Suspended domain name", sample: "example.com" },
      { name: "{{reason}}", description: "Reason for domain suspension", sample: "Terms of service review" },
      { name: "{{contact_email}}", description: "Support or admin contact email", sample: "admin@builtpanel.net" },
      { name: "{{panel_name}}", description: "Platform / branding name", sample: "Built Panel" },
      { name: "{{year}}", description: "Current calendar year", sample: "2026" },
    ],
  },
  {
    id: "tpl_suspended_user",
    type: "suspended_user",
    title: "Account Suspended",
    description: "Served for all sites belonging to a suspended tenant user account.",
    statusCode: 403,
    content: defaultSuspendedUserHtml,
    defaultContent: defaultSuspendedUserHtml,
    updatedAt: "2026-11-10T12:00:00Z",
    isCustomized: false,
    variables: [
      { name: "{{username}}", description: "Suspended user account name", sample: "tenant_user" },
      { name: "{{domain}}", description: "Domain name being accessed", sample: "client-site.com" },
      { name: "{{contact_email}}", description: "Administrative support email", sample: "support@builtpanel.net" },
      { name: "{{panel_name}}", description: "Platform / branding name", sample: "Built Panel" },
      { name: "{{year}}", description: "Current calendar year", sample: "2026" },
    ],
  },
  {
    id: "tpl_error_404",
    type: "error_404",
    title: "404 Not Found",
    description: "Custom 404 page displayed when requested route or file does not exist.",
    statusCode: 404,
    content: defaultError404Html,
    defaultContent: defaultError404Html,
    updatedAt: "2026-11-10T12:00:00Z",
    isCustomized: false,
    variables: [
      { name: "{{domain}}", description: "Host domain name", sample: "my-app.net" },
      { name: "{{path}}", description: "Missing request path", sample: "/old-missing-page" },
      { name: "{{panel_name}}", description: "Platform / branding name", sample: "Built Panel" },
      { name: "{{year}}", description: "Current calendar year", sample: "2026" },
    ],
  },
  {
    id: "tpl_error_502",
    type: "error_502",
    title: "502 Bad Gateway",
    description: "Custom 502 error page displayed when upstream backend or PHP-FPM daemon fails.",
    statusCode: 502,
    content: defaultError502Html,
    defaultContent: defaultError502Html,
    updatedAt: "2026-11-10T12:00:00Z",
    isCustomized: false,
    variables: [
      { name: "{{domain}}", description: "Host domain name", sample: "api-server.org" },
      { name: "{{service}}", description: "Failing upstream daemon name", sample: "php-fpm-8.3" },
      { name: "{{panel_name}}", description: "Platform / branding name", sample: "Built Panel" },
      { name: "{{year}}", description: "Current calendar year", sample: "2026" },
    ],
  },
];
