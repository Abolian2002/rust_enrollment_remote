# Admin Cloudflare Access Runbook

This project has two public-facing web surfaces:

- `cpa.abolian.online`: student/parent chat frontend.
- `admin.cpa.abolian.online`: admissions admin dashboard.

The admin dashboard must not be exposed as a normal public page. Put Cloudflare Access in front of it before publishing the hostname.

## Target Architecture

```text
Browser
  -> Cloudflare Access policy for admin.cpa.abolian.online
  -> Cloudflare Tunnel on Hong Kong server
  -> Hong Kong Nginx 127.0.0.1:8317
  -> SSH reverse tunnels
  -> project server admin preview 127.0.0.1:5173
  -> project server Rust API 127.0.0.1:4000
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
   - Subdomain: `admin.cpa`
   - Domain: `abolian.online`
   - Path: leave empty, or `/` if required by the UI.
4. Add an allow policy for the real administrator emails only.
5. Use One-time PIN or an existing IdP.
6. Copy the application's `AUD tag` for a later backend hardening step.

In Cloudflare Tunnel public hostnames, add:

```text
admin.cpa.abolian.online -> http://127.0.0.1:8317
```

This reuses the existing Hong Kong `cloudflared` process and lets Nginx route by hostname.

## Project Server Admin Frontend

The admin app is built at:

```text
/home/t2_enroll_ai/rust_enrollment/apps/admin/dist
```

Run it with Vite preview on localhost only:

```bash
cd /home/t2_enroll_ai/rust_enrollment/apps/admin
source ~/.nvm/nvm.sh
npm run preview -- --host 127.0.0.1 --port 5173
```

Recommended `.run/admin-web.env`:

```text
PATH=/home/t2_enroll_ai/.nvm/versions/node/v22.22.3/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
VITE_ADMIN_API_BASE_URL=
```

`VITE_ADMIN_API_BASE_URL` should stay empty in production so the browser calls same-origin `/api/v1/admin/*`.

## SSH Tunnel Changes

Existing public chain:

```text
project 3000 -> jump 23000 -> Hong Kong 13000 -> student frontend
project 4000 -> jump 24000 -> Hong Kong 14000 -> Rust API
```

Add admin frontend:

```text
project 5173 -> jump 25173 -> Hong Kong 15173 -> admin frontend
```

On the project-to-jump local tunnel, add:

```text
-L 127.0.0.1:25173:127.0.0.1:5173
```

On the jump-to-Hong-Kong reverse tunnel, add:

```text
-R 127.0.0.1:15173:127.0.0.1:25173
```

Do not modify other Kubernetes or model services.

## Hong Kong Nginx

Add a second server block:

```nginx
server {
    listen 127.0.0.1:8317;
    server_name admin.cpa.abolian.online;

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
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_pass http://127.0.0.1:15173;
    }
}
```

Keep `/api/v1/admin/*` separate from the student `/api/` routes. The student domain should not proxy admin API with a browser-visible token.

## Verification

Before enabling Access publicly:

```bash
curl -fsS http://127.0.0.1:5173/ >/dev/null
curl -fsS -H "Authorization: Bearer $ADMIN_API_TOKEN" \
  http://127.0.0.1:4000/api/v1/admin/dashboard/summary >/dev/null
```

After Cloudflare Access is configured:

```bash
curl -I --max-time 20 https://admin.cpa.abolian.online/
curl -I --max-time 20 https://admin.cpa.abolian.online/api/v1/admin/dashboard/summary
```

Expected unauthenticated result is a Cloudflare Access login or redirect, not raw admin JSON.

After logging in from a browser:

- Dashboard loads real statistics.
- Conversation audit loads real conversations.
- Knowledge page loads FAQ and PDF chunks.

## Later Hardening

The current production-safe minimum is:

- Cloudflare Access protects `admin.cpa.abolian.online`.
- Nginx injects `ADMIN_API_TOKEN` server-side.
- Browser bundle contains no admin API token.

The next hardening step is to validate `Cf-Access-Jwt-Assertion` in Rust using:

- `TEAM_DOMAIN=https://<team>.cloudflareaccess.com`
- `POLICY_AUD=<Access application AUD tag>`

Cloudflare's documented pattern is to validate the JWT signature against the team's public keys and verify issuer and audience before trusting the request.
