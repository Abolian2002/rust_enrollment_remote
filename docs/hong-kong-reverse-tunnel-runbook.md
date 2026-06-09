# 香港公网入口与反向隧道 Runbook

## 当前拓扑

当前已落地的是“三段链路”：

```text
香港服务器 47.86.43.227
  127.0.0.1:13000 / 127.0.0.1:14000
  <- SSH remote forward
Tailscale 跳板机 zgh-eduai 100.95.71.110
  127.0.0.1:23000 / 127.0.0.1:24000
  <- SSH local forward
项目服务器 train-2 10.10.200.13
  127.0.0.1:3000 / 127.0.0.1:4000
```

端口含义：

- 香港 `127.0.0.1:13000` -> 跳板机 `127.0.0.1:23000` -> 项目服务器 `127.0.0.1:3000`，Next.js 前端。
- 香港 `127.0.0.1:14000` -> 跳板机 `127.0.0.1:24000` -> 项目服务器 `127.0.0.1:4000`，Rust Axum API。

后续 Cloudflare/Nginx 应指向香港本机：

- 前端 upstream：`http://127.0.0.1:13000`
- API upstream：`http://127.0.0.1:14000`

## 服务器信息

- 香港服务器：`root@47.86.43.227`
- 香港管理私钥：
  - 新密钥：`/home/scm2002/.ssh/aboabo.pem`
  - 旧密钥：`/home/scm2002/.ssh/xianggang.pem`
- Tailscale 跳板机：`root@100.95.71.110`
- 项目服务器：`t2_enroll_ai@10.10.200.13`

## 已完成清理

香港服务器旧 CPA 服务已清理：

- 停止并禁用：`cliproxyapi.service`
- 删除：`/root/cli-proxy-api`
- 删除：`/root/config.yaml`
- 删除：`/root/.cli-proxy-api`
- 删除：`/etc/systemd/system/cliproxyapi.service`

香港服务器梯子已关闭：

- 停止并禁用：`mihomo.service`
- `127.0.0.1:10090` 已不再监听

旧 CPA Cloudflare Tunnel 已关闭：

- 停止并禁用：`cloudflared.service`
- `cloudflared` 程序仍保留，版本曾确认为 `2026.3.0`

删除前备份位于香港服务器：

```text
/root/cleanup-backup-20260608-210758
```

## SSH Key

跳板机上已生成两把专用 key：

```text
/root/.ssh/zgh_to_project_ed25519
/root/.ssh/zgh_to_hk_tunnel_ed25519
```

用途：

- `zgh_to_project_ed25519`：跳板机免密连接项目服务器 `t2_enroll_ai@10.10.200.13`。
- `zgh_to_hk_tunnel_ed25519`：跳板机连接香港服务器，只用于 remote port forwarding。

香港服务器 `/root/.ssh/authorized_keys` 当前应包含：

- `aboabo-management`
- `xianggang-management`
- `zgh-eduai-to-hk-enrollment-tunnel`

其中 `zgh-eduai-to-hk-enrollment-tunnel` 使用受限权限：

```text
restrict,port-forwarding,permitlisten="127.0.0.1:13000",permitlisten="127.0.0.1:14000"
```

## 持久化服务

两个 systemd 服务部署在 Tailscale 跳板机 `100.95.71.110` 上。

### 1. 跳板机到项目服务器本地隧道

服务名：

```text
enrollment-project-local-tunnel.service
```

文件：

```text
/etc/systemd/system/enrollment-project-local-tunnel.service
```

核心命令：

```bash
/usr/bin/ssh \
  -i /root/.ssh/zgh_to_project_ed25519 \
  -o StrictHostKeyChecking=accept-new \
  -o ExitOnForwardFailure=yes \
  -o ServerAliveInterval=30 \
  -o ServerAliveCountMax=3 \
  -N \
  -L 127.0.0.1:23000:127.0.0.1:3000 \
  -L 127.0.0.1:24000:127.0.0.1:4000 \
  t2_enroll_ai@10.10.200.13
```

### 2. 跳板机到香港服务器反向隧道

服务名：

```text
enrollment-hk-reverse-tunnel.service
```

文件：

```text
/etc/systemd/system/enrollment-hk-reverse-tunnel.service
```

核心命令：

```bash
/usr/bin/ssh \
  -i /root/.ssh/zgh_to_hk_tunnel_ed25519 \
  -o StrictHostKeyChecking=accept-new \
  -o ExitOnForwardFailure=yes \
  -o ServerAliveInterval=30 \
  -o ServerAliveCountMax=3 \
  -N \
  -R 127.0.0.1:13000:127.0.0.1:23000 \
  -R 127.0.0.1:14000:127.0.0.1:24000 \
  root@47.86.43.227
```

## 常用操作

在跳板机查看服务：

```bash
sshpass -p 'qwer123456' ssh root@100.95.71.110 \
  'systemctl --no-pager -l status enrollment-project-local-tunnel.service enrollment-hk-reverse-tunnel.service'
```

重启隧道：

```bash
sshpass -p 'qwer123456' ssh root@100.95.71.110 \
  'systemctl restart enrollment-project-local-tunnel.service enrollment-hk-reverse-tunnel.service'
```

查看跳板机本地监听：

```bash
sshpass -p 'qwer123456' ssh root@100.95.71.110 \
  'ss -ltnp | grep -E ":(23000|24000)"'
```

查看香港服务器监听：

```bash
ssh -i /home/scm2002/.ssh/aboabo.pem root@47.86.43.227 \
  'ss -ltnp | grep -E ":(13000|14000)"'
```

验证香港到前端：

```bash
ssh -i /home/scm2002/.ssh/aboabo.pem root@47.86.43.227 \
  'curl -I --max-time 10 http://127.0.0.1:13000 | head'
```

验证香港到 API：

```bash
ssh -i /home/scm2002/.ssh/aboabo.pem root@47.86.43.227 \
  'curl -fsS --max-time 10 http://127.0.0.1:14000/api/v1/health'
```

当前验证结果应类似：

```text
HTTP/1.1 307 Temporary Redirect
location: /chat

{"success":true,"data":{"database":"ok","service":"rust-enrollment-api","status":"ok"},"meta":{},"error":null}
```

## Cloudflare Tunnel 后续配置

当前已经复用旧 Cloudflare Tunnel：

```text
cpa.abolian.online -> cloudflared on 香港服务器 -> http://127.0.0.1:8317
```

`cloudflared.service` 已启用并运行：

```bash
ssh -i /home/scm2002/.ssh/aboabo.pem root@47.86.43.227 \
  'systemctl status cloudflared --no-pager -l'
```

Cloudflare 日志曾确认远程配置：

```text
hostname: cpa.abolian.online
service: http://127.0.0.1:8317
```

香港服务器已安装并启用 Nginx，配置文件：

```text
/etc/nginx/sites-available/hnu-enrollment-cpa
/etc/nginx/sites-enabled/hnu-enrollment-cpa
```

当前 Nginx 链路：

```text
Cloudflare Tunnel -> http://127.0.0.1:8317 -> Nginx on 香港服务器
Nginx /      -> http://127.0.0.1:13000
Nginx /api/  -> http://127.0.0.1:14000
Nginx /api/v1/chat/voice WebSocket -> http://127.0.0.1:14000
```

Nginx 配置要点：

```nginx
server {
    listen 127.0.0.1:8317;
    server_name cpa.abolian.online;

    client_max_body_size 20m;
    proxy_connect_timeout 10s;
    proxy_send_timeout 3600s;
    proxy_read_timeout 3600s;

    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;

    location /api/ {
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_buffering off;
        proxy_pass http://127.0.0.1:14000;
    }

    location /_next/ {
        proxy_pass http://127.0.0.1:13000;
    }

    location / {
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_pass http://127.0.0.1:13000;
    }
}
```

项目服务器前端环境已改为：

```text
NEXT_PUBLIC_API_BASE_URL=https://cpa.abolian.online
```

位置：

```text
/home/t2_enroll_ai/rust_enrollment/apps/web/.env.local
```

注意：前端当前是 `npm run dev -- --hostname 127.0.0.1 --port 3000` 方式运行。项目服务器没有全局 Node/NPM，启动时需要显式 PATH：

```bash
export PATH=/home/t2_enroll_ai/.nvm/versions/node/v22.22.3/bin:$PATH
```

公网验证命令：

```bash
curl -I --max-time 20 https://cpa.abolian.online/chat
curl -fsS --max-time 20 https://cpa.abolian.online/api/v1/health
```

WebSocket 验证：

```bash
node - <<'NODE'
const ws = new WebSocket('wss://cpa.abolian.online/api/v1/chat/voice');
const timer = setTimeout(() => process.exit(1), 15000);
ws.addEventListener('open', () => {
  console.log('ws_open');
  clearTimeout(timer);
  ws.close();
});
ws.addEventListener('error', (event) => {
  console.error(event.message || event);
  clearTimeout(timer);
  process.exit(1);
});
NODE
```

当前验证结果：

- `https://cpa.abolian.online/chat` 返回 `HTTP/2 200`
- `https://cpa.abolian.online/api/v1/health` 返回 database `ok`
- `wss://cpa.abolian.online/api/v1/chat/voice` 握手成功

这种方式可以统一处理：

- API 路径转发
- WebSocket upgrade
- 超时
- 访问日志
- 后续灰度或备用 upstream
