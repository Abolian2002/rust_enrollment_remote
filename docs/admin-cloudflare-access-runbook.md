# Admin Cloudflare Access Runbook

This project has two public-facing web surfaces:

- `cpa.abolian.online`: student/parent chat frontend.
- `admin.abolian.online`: admissions admin dashboard.

The admin dashboard must not be exposed as a normal public page. Put Cloudflare Access in front of it before publishing the hostname.

## Target Architecture

```text
Browser
  -> Cloudflare Access policy for admin.abolian.online
  -> Cloudflare Tunnel on Hong Kong server
  -> Hong Kong Nginx 127.0.0.1:8317
  -> static admin files in /var/www/hnu-enrollment-admin
  -> project server Rust API through existing Hong Kong 127.0.0.1:14000 tunnel
```

Nginx should keep the admin API token server-side:

```text
browser JS       -> /api/v1/admin/*
Hong Kong Nginx  -> adds Authorization: Bearer <ADMIN_API_TOKEN>
Rust API         -> validates ADMIN_API_TOKEN
```

Do not build `VITE_ADMIN_API_TOKEN` into the public admin frontend for production. It is only acceptable for local development.

## Cloudflare Setup

In Cloudflare Zero Trust:

1. Go to `Access controls` -> `Applications`.
2. Create a `Self-hosted` application.
3. Add public hostname:
   - Subdomain: `admin`
   - Domain: `abolian.online`
   - Path: leave empty, or `/` if required by the UI.
4. Add an allow policy for the real administrator emails only.
5. Use One-time PIN or an existing IdP.
6. Copy the application's `AUD tag` for a later backend hardening step.

In Cloudflare Tunnel public hostnames, add:

```text
admin.abolian.online -> http://127.0.0.1:8317
```

This reuses the existing Hong Kong `cloudflared` process and lets Nginx route by hostname.

As of 2026-06-10, the existing `cpa` tunnel configuration has already been updated through the Cloudflare API:

```text
cpa.abolian.online       -> http://127.0.0.1:8317
admin.abolian.online -> http://127.0.0.1:8317
catch-all                -> http_status:404
```

As of 2026-06-10, DNS and Access have also been configured through a temporary Cloudflare API token:

```text
admin.abolian.online CNAME cc120d3e-7559-40dc-9fb2-a7934bb13575.cfargotunnel.com proxied=true
Access application: HNU Enrollment Admin
Access policy: allow cunmingsong2002@gmail.com via One-time PIN
```

Revoke the temporary Cloudflare API token after verification.

## Admin Static Frontend

The admin app is built at:

```text
/home/t2_enroll_ai/rust_enrollment/apps/admin/dist
```

Deploy the built files to the Hong Kong Nginx static root:

```bash
cd /home/t2_enroll_ai/rust_enrollment/apps/admin
tar -czf /tmp/hnu-admin-dist.tgz -C dist .
scp /tmp/hnu-admin-dist.tgz root@47.86.43.227:/tmp/hnu-admin-dist.tgz
ssh root@47.86.43.227 '
  rm -rf /var/www/hnu-enrollment-admin.new
  mkdir -p /var/www/hnu-enrollment-admin.new
  tar -xzf /tmp/hnu-admin-dist.tgz -C /var/www/hnu-enrollment-admin.new
  rm -rf /var/www/hnu-enrollment-admin.prev
  [ -d /var/www/hnu-enrollment-admin ] && mv /var/www/hnu-enrollment-admin /var/www/hnu-enrollment-admin.prev
  mv /var/www/hnu-enrollment-admin.new /var/www/hnu-enrollment-admin
'
```

As of 2026-06-10, this has been deployed and verified on the Hong Kong server:

```text
/var/www/hnu-enrollment-admin
```

`VITE_ADMIN_API_BASE_URL` should stay empty in production so the browser calls same-origin `/api/v1/admin/*`.

## SSH Tunnel Changes

Existing public chain:

```text
project 3000 -> jump 23000 -> Hong Kong 13000 -> student frontend
project 4000 -> jump 24000 -> Hong Kong 14000 -> Rust API
```

No new SSH tunnel is required for the admin frontend because the admin app is served as static files on the Hong Kong Nginx server. The admin API reuses the existing `14000 -> 24000 -> 4000` API tunnel.

Note: Hong Kong `/root/.ssh/authorized_keys` was prepared for a possible `15173` reverse port during an earlier approach:

```text
permitlisten="127.0.0.1:13000"
permitlisten="127.0.0.1:14000"
permitlisten="127.0.0.1:15173"
```

This extra permission is currently unused. It can be removed later if the static-hosting approach remains final.

## Hong Kong Nginx

Add a second server block:

```nginx
server {
    listen 127.0.0.1:8317;
    server_name admin.abolian.online;

    client_max_body_size 20m;
    proxy_connect_timeout 10s;
    proxy_send_timeout 3600s;
    proxy_read_timeout 3600s;

    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;

    location /api/v1/admin/ {
        proxy_http_version 1.1;
        proxy_set_header Authorization "Bearer REPLACE_WITH_ADMIN_API_TOKEN";
        proxy_pass http://127.0.0.1:14000;
    }

    location / {
        root /var/www/hnu-enrollment-admin;
        try_files $uri $uri/ /index.html;
    }
}
```

Keep `/api/v1/admin/*` separate from the student `/api/` routes. The student domain should not proxy admin API with a browser-visible token.

## Verification

Before enabling Access publicly:

```bash
ssh root@47.86.43.227 \
  'curl -fsS -H "Host: admin.abolian.online" http://127.0.0.1:8317/ >/dev/null'

ssh root@47.86.43.227 \
  'curl -fsS -H "Host: admin.abolian.online" http://127.0.0.1:8317/api/v1/admin/dashboard/summary >/dev/null'
```

After Cloudflare Access is configured:

```bash
curl -I --max-time 20 https://admin.abolian.online/
curl -I --max-time 20 https://admin.abolian.online/api/v1/admin/dashboard/summary
```

Expected unauthenticated result is a Cloudflare Access login or redirect, not raw admin JSON.

After logging in from a browser:

- Dashboard loads real statistics.
- Conversation audit loads real conversations.
- Knowledge page loads FAQ and PDF chunks.

## Later Hardening

The current production-safe minimum is:

- Cloudflare Access protects `admin.abolian.online`.
- Nginx injects `ADMIN_API_TOKEN` server-side.
- Browser bundle contains no admin API token.

The next hardening step is to validate `Cf-Access-Jwt-Assertion` in Rust using:

- `TEAM_DOMAIN=https://<team>.cloudflareaccess.com`
- `POLICY_AUD=<Access application AUD tag>`

Cloudflare's documented pattern is to validate the JWT signature against the team's public keys and verify issuer and audience before trusting the request.
