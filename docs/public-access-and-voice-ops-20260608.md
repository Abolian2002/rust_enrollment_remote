# 公网访问、香港中转与语音服务恢复记录

日期：2026-06-08

## 当前公网访问链路

当前公网域名：

```text
https://cpa.abolian.online/chat
```

当前链路：

```text
用户浏览器
  -> cpa.abolian.online
  -> Cloudflare Tunnel
  -> 香港服务器 cloudflared
  -> 香港 Nginx 127.0.0.1:8317
  -> SSH remote forward 127.0.0.1:13000 / 14000
  -> Tailscale 跳板机 100.95.71.110
  -> SSH local forward 127.0.0.1:23000 / 24000
  -> 项目服务器 10.10.200.13
  -> 前端 127.0.0.1:3000 / 后端 127.0.0.1:4000
```

端口对应关系：

```text
香港 127.0.0.1:13000 -> 跳板机 127.0.0.1:23000 -> 项目服务器 127.0.0.1:3000
香港 127.0.0.1:14000 -> 跳板机 127.0.0.1:24000 -> 项目服务器 127.0.0.1:4000
香港 127.0.0.1:8317  -> Nginx 统一入口
```

香港 Nginx：

```text
/etc/nginx/sites-available/hnu-enrollment-cpa
/etc/nginx/sites-enabled/hnu-enrollment-cpa
```

Nginx 转发规则：

```text
/      -> http://127.0.0.1:13000
/_next -> http://127.0.0.1:13000
/api/  -> http://127.0.0.1:14000
```

Cloudflare Tunnel：

```text
cloudflared.service
cpa.abolian.online -> http://127.0.0.1:8317
```

## 本轮实际修改

### 香港服务器

已安装并启用 Nginx：

```bash
apt-get update
apt-get install -y nginx
systemctl enable --now nginx
```

已启用旧的 CPA Cloudflare Tunnel：

```bash
systemctl enable --now cloudflared
```

验证：

```bash
ssh -i /home/scm2002/.ssh/aboabo.pem root@47.86.43.227 \
  'systemctl is-active cloudflared nginx; ss -ltnp | grep -E ":(8317|13000|14000)"'
```

### 项目服务器前端

项目服务器：

```text
t2_enroll_ai@10.10.200.13
```

前端环境改为公网域名：

```text
/home/t2_enroll_ai/rust_enrollment/apps/web/.env.local
NEXT_PUBLIC_API_BASE_URL=https://cpa.abolian.online
```

注意：项目服务器没有全局 `node/npm`，需要显式 PATH：

```bash
export PATH=/home/t2_enroll_ai/.nvm/versions/node/v22.22.3/bin:$PATH
```

前端重启命令：

```bash
cd /home/t2_enroll_ai/rust_enrollment
if [ -f .run/web.pid ]; then kill "$(cat .run/web.pid)" 2>/dev/null || true; fi
nohup bash -lc 'export PATH=/home/t2_enroll_ai/.nvm/versions/node/v22.22.3/bin:$PATH; cd /home/t2_enroll_ai/rust_enrollment/apps/web && npm run dev -- --hostname 127.0.0.1 --port 3000' > .run/web.log 2>&1 &
echo $! > .run/web.pid
```

## 语音不可用的原因与修复

现象：

- 页面显示“语音暂不可用”
- 公网 WebSocket 能连通，但收到 `tts_error`
- `wss://cpa.abolian.online/api/v1/chat/voice` 能握手，文本 chunk 正常，但没有音频 chunk

关键日志：

```text
failed to request local streaming TTS: error sending request for url (http://127.0.0.1:50000/v1/audio/stream)
```

原因：

```text
项目服务器上的模型服务入口 model-lb / embedding / CosyVoice 进程已退出。
Qwen llama-server 仍在运行，但 18080/8114/50000 负载均衡入口和 CosyVoice 子服务不在监听。
```

本次没有修改 Rust 语音取消链路，没有改 `apps/api/src/main.rs`。

修复方式：只恢复缺失的模型服务边缘进程，不重启 Qwen llama-server。

恢复的端口：

```text
18080 -> Qwen load balancer
8114  -> embedding load balancer
50000 -> CosyVoice load balancer
8115/8116 -> embedding workers
50001/50002 -> CosyVoice workers
```

恢复脚本逻辑：

```bash
BASE=/home/t2_enroll_ai
LOGDIR="$BASE/model-service-logs"
RUNDIR="$BASE/model-service-run"
EMBED_DIR="$BASE/embedding-model"
COSY_DIR="$BASE/cosyvoice3"
COSY_MODEL="$COSY_DIR/pretrained_models/Fun-CosyVoice3-0.5B"
LB_SCRIPT="$BASE/model-service-stack/model_lb.py"

# 只停缺失边缘服务，不停 Qwen llama-server
pgrep -f 'uvicorn app:app --host 0.0.0.0 --port 811' | xargs -r kill
pgrep -f 'runtime/python/fastapi/server.py --port 500' | xargs -r kill
pgrep -f 'model_lb.py' | xargs -r kill

cd "$EMBED_DIR"
nohup env CUDA_VISIBLE_DEVICES=6 EMBEDDING_DEVICE=cuda EMBEDDING_BATCH_SIZE=16 ./.venv/bin/uvicorn app:app --host 0.0.0.0 --port 8115 > "$LOGDIR/qwen3-embedding-0.log" 2>&1 &
nohup env CUDA_VISIBLE_DEVICES=7 EMBEDDING_DEVICE=cuda EMBEDDING_BATCH_SIZE=16 ./.venv/bin/uvicorn app:app --host 0.0.0.0 --port 8116 > "$LOGDIR/qwen3-embedding-1.log" 2>&1 &

cd "$COSY_DIR"
nohup env CUDA_VISIBLE_DEVICES=8 ./.venv/bin/python runtime/python/fastapi/server.py --port 50001 --model_dir "$COSY_MODEL" > "$LOGDIR/cosyvoice3-0.log" 2>&1 &
nohup env CUDA_VISIBLE_DEVICES=9 ./.venv/bin/python runtime/python/fastapi/server.py --port 50002 --model_dir "$COSY_MODEL" > "$LOGDIR/cosyvoice3-1.log" 2>&1 &

nohup python3 "$LB_SCRIPT" > "$LOGDIR/model-lb.log" 2>&1 &
echo $! > "$LOGDIR/model-lb.pid"
```

验证：

```bash
ss -ltnp | grep -E '(:18080|:8114|:50000|:8115|:8116|:50001|:50002)'
```

公网语音验证：

```bash
node - <<'NODE'
const ws = new WebSocket('wss://cpa.abolian.online/api/v1/chat/voice');
let audio = 0;
const timer = setTimeout(() => process.exit(1), 45000);
ws.addEventListener('open', () => {
  ws.send(JSON.stringify({ message: '你好你是谁', conversationId: `voice-check-${Date.now()}` }));
});
ws.addEventListener('message', (event) => {
  if (typeof event.data !== 'string') {
    audio += 1;
    if (audio === 1) console.log('first_audio');
    return;
  }
  if (event.data.includes('"event":"done"')) {
    console.log('audio_chunks', audio);
    clearTimeout(timer);
    ws.close();
    process.exit(audio > 0 ? 0 : 2);
  }
});
NODE
```

当前验证结果：

```text
first_audio
audio_chunks 144
```

## 不要破坏的语音取消链路

之前 Rust API 已修复：

- 同一 `conversationId` 新语音请求会取消旧语音 session。
- 客户端断开会取消 agent/TTS 分段任务。
- 取消时 abort 后端分段器和 TTS 合成任务。
- 正常完成时仍等待 TTS 队列播完后再发 `done`。

本轮只恢复模型服务，不要回退这些逻辑。

相关代码：

```text
/home/scm2002/Code/rust_enrollment/apps/api/src/main.rs
```

## 将来不用香港中转时怎么改回公网 IP

如果后续项目服务器有公网 IP，推荐改成：

```text
Cloudflare DNS / Tunnel -> 项目服务器公网入口 -> 项目服务器 Nginx -> 127.0.0.1:3000 / 127.0.0.1:4000
```

需要做的改动：

1. 在项目服务器上部署 Nginx，监听公网 HTTPS 或 Cloudflare 回源端口。
2. Nginx 规则保持一致：

```text
/      -> http://127.0.0.1:3000
/_next -> http://127.0.0.1:3000
/api/  -> http://127.0.0.1:4000
```

3. 前端环境改为新的公网域名：

```text
NEXT_PUBLIC_API_BASE_URL=https://新的域名
```

4. 重启前端，使 Next.js 重新读取 `NEXT_PUBLIC_API_BASE_URL`。
5. Cloudflare DNS 改为指向项目服务器公网 IP，或新建 tunnel 到项目服务器。
6. 停用香港中转相关服务：

在跳板机：

```bash
systemctl disable --now enrollment-project-local-tunnel.service
systemctl disable --now enrollment-hk-reverse-tunnel.service
```

在香港服务器：

```bash
systemctl disable --now cloudflared
systemctl disable --now nginx
```

如果香港服务器还有其他用途，不要删除密钥和备份，只停服务即可。

## 当前公网验证命令

```bash
curl -I --max-time 20 https://cpa.abolian.online/chat
curl -fsS --max-time 20 https://cpa.abolian.online/api/v1/health
```

期望：

```text
HTTP/2 200
database: ok
```
