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

## 2026-06-09 语音并发与取消链路修复

多人同时访问时出现过“首音之后没有声音”的现象。日志显示根因不是单纯浏览器播放问题，而是几处并发策略叠加：

- Rust API 的全局 TTS semaphore 只有 2 个许可，且所有段落统一排队 20 秒；后续段落在高并发下会被直接丢弃。
- `model_lb.py` 在客户端断开时把 `BrokenPipeError` 当成 upstream 失败并重试下一个 CosyVoice worker，导致用户切换问题后旧语音可能继续占用 worker。
- CosyVoice FastAPI worker 内部允许同一个模型对象同时被多个请求迭代，RTF 会被拉高，首包和后续段都会变慢。

已做的修复：

1. Rust API：
   - `SERVER_TTS_MAX_CONCURRENT_SYNTH=4`
   - `SERVER_TTS_FIRST_QUEUE_TIMEOUT_MS=20000`
   - `SERVER_TTS_CONTINUATION_QUEUE_TIMEOUT_MS=120000`
   - 首段保持快速失败，后续段落允许更长等待，并且仍可被新问题取消。
   - TTS 日志增加 `segment_index` 和 `is_first_segment`，方便定位是否是首段还是后续段排队。

2. `model_lb.py`：
   - CosyVoice upstream 扩到 `50001/50002/50003/50004`。
   - downstream 断开时关闭当前 upstream 连接并返回，不再 retry 其他 worker。
   - upstream 选择从简单轮询改为 least-inflight，优先选择当前负载更低的 worker。

3. CosyVoice FastAPI：
   - `/tts_stream` 的 streaming generator 检查 `request.is_disconnected()`。
   - 客户端取消时尝试 close 生成器。
   - 每个 worker 内加 `INFERENCE_LOCK`，保证单 worker 内模型推理串行；总并发由多个 worker 承接。

4. 模型服务运行状态：
   - Qwen：`18081/18082/18083`，每个 `--parallel 1`，总 LLM 并发约 3。
   - Embedding：`8115/8116`，LB 入口 `8114`。
   - CosyVoice：`50001/50003` 在 GPU 8，`50002/50004` 在 GPU 9，LB 入口 `50000`。
   - CosyVoice 每张 A100 约 7.8GB 显存，显存不是瓶颈。

验证命令：

```bash
curl -fsS http://127.0.0.1:4000/api/v1/health
ss -ltnp | grep -E '(:18080|:8114|:50000|:50001|:50002|:50003|:50004|:4000)'
nvidia-smi --query-gpu=index,memory.used,memory.total,utilization.gpu --format=csv,noheader,nounits
tail -120 /home/t2_enroll_ai/rust_enrollment/.run/api.log
tail -120 /home/t2_enroll_ai/model-service-logs/model-lb.log
```

直接 TTS 流烟测：

```bash
python3 - <<'PY'
import requests, time
url = 'http://127.0.0.1:50000/v1/audio/stream'
t0 = time.time()
r = requests.post(url, json={'input': '你好，我是哈尔滨师范大学招生智能顾问，很高兴为你解答问题。'}, stream=True, timeout=60)
print('status', r.status_code, 'upstream', r.headers.get('X-Model-Upstream'))
for chunk in r.iter_content(4096):
    if chunk:
        print('first_audio_s', round(time.time() - t0, 3))
        break
r.close()
PY
```

取消链路验证点：

```text
[cosyvoice] client disconnected while proxying to 127.0.0.1:5000x; not retrying
```

如果再次出现“首音后静音”，优先检查：

- API 日志是否还有 `server-side voice TTS queue wait timed out`。
- LB 日志是否又出现大量 502 或 BrokenPipe traceback。
- CosyVoice worker 日志的 RTF 是否持续大于 5。
- 是否有旧请求没有被取消，导致 4 个 worker 长时间全满。

LLM 并发说明：

当前大模型服务实际并发约为 3，因为三路 llama-server 都是 `--parallel 1`。Rust API 的 `CHAT_MAX_CONCURRENT_REQUESTS` 是 HTTP/WS 请求闸门，不等同于 LLM 并发闸门。后续如果几十个家长同时访问，建议新增独立的 LLM stage semaphore，或把 llama-server `--parallel` 小步提升到 2 并压测首 token、总耗时和显存，再决定是否扩大。

## 2026-06-09 LLM 并发控制策略

目标是支持约 10-20 位家长同时访问，同时不破坏当前回答质量、首 token 速度和语音链路。

当前选择：

- 保留 `CHAT_MAX_CONCURRENT_REQUESTS` 作为入口级总请求闸门。
- 新增独立 LLM synthesis 闸门：
  - `LLM_MAX_CONCURRENT_REQUESTS=3`
  - `LLM_QUEUE_TIMEOUT_MS=20000`
- 只限制真正调用模型的 `llm.complete()` / `llm.stream_complete()`。
- 不限制数据库、FAQ、Excel、PDF chunk、embedding、TTS。

原因：

- 当前 Qwen 服务是 3 个 `llama-server` 实例，每个 `--parallel 1`，实际 LLM 并发约 3。
- 如果只提高 `CHAT_MAX_CONCURRENT_REQUESTS`，LLM 请求会在模型服务内部堆积，首 token 变慢，也会拖慢语音首音。
- 如果直接把 `--parallel` 提到 2，可能改变单请求速度、KV cache 显存、输出稳定性和首 token；必须压测后再决定。

推荐线上初始配置：

```text
CHAT_MAX_CONCURRENT_REQUESTS=20
LLM_MAX_CONCURRENT_REQUESTS=3
LLM_QUEUE_TIMEOUT_MS=20000
```

观测指标：

- API 日志中的 `llm concurrency permit acquired`：
  - `queue_wait_ms` 长期接近 0：LLM 容量充足。
  - `queue_wait_ms` 经常超过 5000：LLM 开始成为瓶颈。
  - 出现 `llm concurrency queue wait timed out`：并发超出当前模型容量。
- 用户侧首 token 与语音 first audio 是否明显变慢。
- Qwen 三个 llama-server 的 GPU 利用率、显存和请求耗时。

下一步调优顺序：

1. 先保持 `LLM_MAX_CONCURRENT_REQUESTS=3`，压测 10/15/20 位并发。
2. 如果 LLM 队列等待过高，再单独测试其中一路 llama-server 的 `--parallel 2`。
3. `--parallel 2` 稳定后，再把 `LLM_MAX_CONCURRENT_REQUESTS` 小步提高到 4 或 5。
4. 不建议直接把 LLM 并发拉到 20；20 位访客并不等于 20 个同时生成中的 LLM 请求。
