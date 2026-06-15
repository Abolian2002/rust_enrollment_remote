# 招生智能体项目运维交接现状（2026-06-14）

> 目的：给后续接手的 agent / 开发者快速理解当前本地仓库、内网 GPU 服务器、K8s/PostgreSQL、模型服务、香港公网入口、Cloudflare 配置和已知风险。
>
> 本文包含敏感运维信息。不要公开发布，不要提交到公开仓库，必要时先脱敏。

## 1. 当前结论

- 当前生产访问主链路是：浏览器 -> Cloudflare Tunnel -> 香港服务器 Nginx -> SSH 反向/正向隧道 -> 内网 GPU 项目服务器。
- 学生/家长端公网入口：`https://cpa.abolian.online/chat`，本次验证返回 `HTTP/2 200`。
- 后台管理端公网入口：`https://admin.abolian.online/`，本次验证返回 Cloudflare Access `HTTP/2 302` 登录跳转，说明 Access 保护生效。
- 内网项目服务器 `10.10.200.13` 上 Rust API、Next.js 前端、3 个 LLM worker、embedding、4 个 CosyVoice worker 都在运行。
- 项目数据库是 K8s namespace `hnu-enrollment` 下的 PostgreSQL 16 + pgvector，pod 为 `hnu-enrollment-postgres-0`，宿主机通过 `kubectl port-forward` 暴露到 `127.0.0.1:55432`。
- 当前本地和服务器仓库都有 3 个未提交的后台管理前端文件改动：`apps/admin/src/App.tsx`、`apps/admin/src/index.css`、`apps/admin/src/types/admin.ts`。这些改动是后台管理真实数据/布局相关，不是语音、agent 或聊天链路。
- `127.0.0.1:10090` 曾在内网项目服务器上监听；本次确认它来自本地 WSL 的 SSH 反向隧道 `-R 10090:127.0.0.1:10090`，不是招生项目业务链路，已停止。

## 2. 仓库与路径

### 本地仓库

- 路径：`/home/scm2002/Code/rust_enrollment`
- 分支：`main`
- remotes：
  - `origin https://github.com/Abolian2002/rust_enrollment.git`
  - `enrollment_remote https://github.com/Abolian2002/rust_enrollment_remote.git`
- 本次确认的最近提交：
  - `206716b add public ticket submission flow`
  - `785a72f integrate admin data and faq knowledge`
  - `0449638 document admin access public endpoint`
  - `1954501 document admin access tunnel setup`
  - `c1f8b4e prepare admin cloudflare access deployment`
- 本地未提交文件：
  - `apps/admin/src/App.tsx`
  - `apps/admin/src/index.css`
  - `apps/admin/src/types/admin.ts`
- 未提交改动性质：
  - 去掉后台管理系统假数据兜底。
  - 增加真实数据读取失败提示、空状态、刷新按钮。
  - 调整知识库管理页布局，避免图表遮挡文字。
  - FAQ 分类覆盖图单独成行居中展示。

### 内网项目服务器仓库

- SSH：`t2_enroll_ai@10.10.200.13`
- 密码：`qwer123456`
- hostname：`train-2`
- 项目路径：`/home/t2_enroll_ai/rust_enrollment`
- 远端仓库：`origin https://github.com/Abolian2002/rust_enrollment_remote.git`
- 服务器仓库 HEAD 与本地一样，最近提交同为 `206716b`。
- 服务器仓库也有同样 3 个未提交后台管理前端文件。

## 3. 服务拓扑

```text
用户浏览器
  -> Cloudflare DNS / Access / Tunnel
  -> 香港服务器 47.86.43.227
     - cloudflared.service
     - Nginx 127.0.0.1:8317
     - SSH remote forward 127.0.0.1:13000 / 14000
  -> Tailscale 跳板机 100.95.71.110
     - enrollment-project-local-tunnel.service
     - enrollment-hk-reverse-tunnel.service
     - 127.0.0.1:23000 / 24000
  -> 内网项目服务器 10.10.200.13
     - Next.js 前端 127.0.0.1:3000
     - Rust API 0.0.0.0:4000
     - local model services
     - K8s PostgreSQL port-forward 127.0.0.1:55432
```

端口映射：

```text
香港 127.0.0.1:13000 -> 跳板机 127.0.0.1:23000 -> 项目服务器 127.0.0.1:3000
香港 127.0.0.1:14000 -> 跳板机 127.0.0.1:24000 -> 项目服务器 127.0.0.1:4000
香港 127.0.0.1:8317  -> Nginx 统一入口
```

## 4. 服务器与访问方式

### 内网项目服务器

- 地址：`10.10.200.13`
- 用户：`t2_enroll_ai`
- 密码：`qwer123456`
- 项目目录：`/home/t2_enroll_ai/rust_enrollment`
- 主要用途：
  - Rust API
  - Next.js 学生端前端
  - PostgreSQL port-forward
  - LLM / embedding / CosyVoice 模型服务

局域网内直连：

```bash
sshpass -p 'qwer123456' ssh t2_enroll_ai@10.10.200.13
```

如果不在局域网，但能访问 Tailscale 跳板机，需要先经跳板机再到项目服务器。当前跳板机的 Tailscale SSH 可能会要求浏览器二次认证。

### Tailscale 跳板机

- 地址：`100.95.71.110`
- 用户：`root`
- 密码：`qwer123456`
- 作用：
  - 连接内网项目服务器。
  - 维持到香港服务器的反向隧道。
- 已知 systemd 服务：
  - `enrollment-project-local-tunnel.service`
  - `enrollment-hk-reverse-tunnel.service`

> 本次尝试直接 SSH 跳板机时，Tailscale 提示需要访问 `https://login.tailscale.com/a/...` 完成额外认证。后续如果连接失败，先处理 Tailscale 认证。

### 香港服务器

- 地址：`47.86.43.227`
- 用户：`root`
- 当前管理 key：`/home/scm2002/.ssh/aboabo.pem`
- 旧 key：`/home/scm2002/.ssh/xianggang.pem`
- 作用：
  - 运行 `cloudflared.service`。
  - 运行 Nginx，监听 `127.0.0.1:8317`。
  - 接收跳板机 remote forward 端口 `127.0.0.1:13000` 和 `127.0.0.1:14000`。
  - 直接托管后台管理系统静态文件。
- 后台管理静态根目录：`/var/www/hnu-enrollment-admin`
- 本次验证静态文件：
  - `/var/www/hnu-enrollment-admin/index.html`
  - `/var/www/hnu-enrollment-admin/assets/index-DrQgHuqf.js`
  - `/var/www/hnu-enrollment-admin/assets/index-D9i87J4P.css`

连接命令：

```bash
ssh -i /home/scm2002/.ssh/aboabo.pem root@47.86.43.227
```

## 5. Cloudflare 配置

### 域名与入口

- 学生/家长端：`https://cpa.abolian.online/chat`
- 后台管理端：`https://admin.abolian.online/`

### Tunnel / Nginx 关系

香港服务器上：

- `cloudflared.service` 正在运行。
- Nginx 正在运行。
- `cloudflared` 将 Cloudflare 公网流量转到香港本机 `127.0.0.1:8317`。
- Nginx 根据域名和路径分流：
  - `cpa.abolian.online`：
    - `/api/*` -> `http://127.0.0.1:14000` -> Rust API
    - 其他前端路径 -> `http://127.0.0.1:13000` -> Next.js
  - `admin.abolian.online`：
    - 静态文件 -> `/var/www/hnu-enrollment-admin`
    - `/api/v1/admin/*` -> `http://127.0.0.1:14000`
    - Nginx 注入后台管理 API 所需的 `Authorization: Bearer <ADMIN_API_TOKEN>`，不要把 admin token 放到浏览器 JS。

本次验证：

- `curl -I https://cpa.abolian.online/chat` 返回 `HTTP/2 200`。
- `curl -I https://admin.abolian.online/` 返回 `HTTP/2 302`，跳到 Cloudflare Access 登录页。

### Cloudflare Access

- 后台管理当前由 Cloudflare Access 保护。已配置过的允许邮箱包括：

```text
cunmingsong2002@gmail.com
zhou_ghui@163.com
758194209@qq.com
shikongjianlun@163.com
```
- 如果后台管理访问要求邮箱验证码，这是 Access 正常行为，不是应用登录异常。

### Cloudflare API token

- 本地 token 文件路径：`/home/scm2002/.secrets/cloudflare_token`
- 文件权限：`600`
- 本次确认文件大小：`54` bytes
- 本次确认 SHA-256：`248537bb86dee3395b550f15faaf16bb932bfbd192552e1d358b824fedb054fd`
- 不要把 token 明文写入仓库或发到聊天里。
- 使用时建议：

```bash
export CLOUDFLARE_API_TOKEN="$(cat /home/scm2002/.secrets/cloudflare_token)"
```

## 6. 内网项目服务器进程与端口

本次在 `train-2` 上直接验证：

| 端口 | 地址 | 进程/用途 | 备注 |
| --- | --- | --- | --- |
| `3000` | `127.0.0.1` | `next-server (v15.5.19)` | 学生/家长端 Next.js，pid `1014496` |
| `4000` | `0.0.0.0` | `./target/release/api` | Rust Axum API，pid `1014351` |
| `55432` | `127.0.0.1` | `kubectl port-forward` | K8s PostgreSQL 到宿主机，pid `3548470` |
| `18080` | `0.0.0.0` | `model_lb.py` | LLM load balancer |
| `18081` | `0.0.0.0` | `llama-server` | Qwen worker 1 |
| `18082` | `0.0.0.0` | `llama-server` | Qwen worker 2 |
| `18083` | `0.0.0.0` | `llama-server` | Qwen worker 3 |
| `8114` | `0.0.0.0` | `model_lb.py` | embedding LB |
| `8116` | `0.0.0.0` | `uvicorn app:app` | embedding worker |
| `50000` | `0.0.0.0` | `model_lb.py` | CosyVoice LB |
| `50001-50004` | `0.0.0.0` | `runtime/python/fastapi/server.py` | 4 个 CosyVoice worker |
| `7860` | `0.0.0.0` | `python` | 非本项目核心链路，GPU 6 占用较高，需另行确认用途 |
| `10090` | `127.0.0.1` / `::1` | 已停止 | 原因是本地 WSL SSH 反向隧道，不是项目业务链路 |
| `5173` | `127.0.0.1` | `node` | 可能是后台管理 Vite dev server，非公网主链路 |

API 健康检查本次通过：

```bash
curl -fsS http://127.0.0.1:4000/api/v1/health
```

返回摘要：

```json
{"success":true,"data":{"database":"ok","service":"rust-enrollment-api","status":"ok"},"meta":{},"error":null}
```

## 7. 模型服务现状

### LLM

- 运行方式：`llama.cpp/build/bin/llama-server`
- 模型路径：
  - `/home/t2_enroll_ai/models/Qwen3.6-35B-A3B-MTP-GGUF/BF16/Qwen3.6-35B-A3B-BF16-00001-of-00002.gguf`
- alias：`qwen3.6-35b-a3b-bf16`
- 端口：
  - `18081`
  - `18082`
  - `18083`
- LB 入口：`18080`
- 关键参数：
  - `--parallel 1`
  - `-c 8192`
  - `-ngl 999`
  - `--split-mode layer`
  - `--jinja`
  - `--reasoning off`
  - `--reasoning-budget 0`
  - `--api-key ragflow-local-qwen`
- Rust API 当前环境：
  - `LLM_PROVIDER=openai-compatible`
  - `LLM_MAX_CONCURRENT_REQUESTS=3`
  - `LLM_QUEUE_TIMEOUT_MS=20000`

### Embedding

- LB 入口：`http://127.0.0.1:8114/v1/embeddings`
- worker：`0.0.0.0:8116`
- 进程：
  - `/home/t2_enroll_ai/embedding-model/.venv/bin/uvicorn app:app --host 0.0.0.0 --port 8116`
- Rust API 当前环境：
  - `LOCAL_EMBEDDING_BASE_URL=http://127.0.0.1:8114/v1/embeddings`
  - `LOCAL_EMBEDDING_MODEL=bge-m3`

> 旧文档里曾提到 `8115/8116` 两个 worker；本次实况只看到 `8116` 监听。若后续 embedding 并发不足，先确认是否需要恢复第二个 worker。

### CosyVoice

- LB 入口：
  - `LOCAL_TTS_STREAM_URL=http://127.0.0.1:50000/v1/audio/stream`
  - `LOCAL_TTS_SPEECH_URL=http://127.0.0.1:50000/v1/audio/speech`
- worker 端口：
  - `50001`
  - `50002`
  - `50003`
  - `50004`
- 模型目录：
  - `/home/t2_enroll_ai/cosyvoice3/pretrained_models/Fun-CosyVoice3-0.5B`
- 重要链路约束：
  - 前端使用 server-side voice WebSocket。
  - 新问题发送后必须取消旧回答的语音合成与播放。
  - 不要破坏“Rust 侧取消 -> LB/worker 停止旧请求 -> 前端停止旧音频”的取消链路。
  - 不要恢复“前端等整段文本再 TTS”的旧模式。

Rust API 当前语音并发相关环境：

```text
SERVER_TTS_MAX_CONCURRENT_SYNTH=4
SERVER_TTS_FIRST_QUEUE_TIMEOUT_MS=20000
SERVER_TTS_CONTINUATION_QUEUE_TIMEOUT_MS=120000
LOCAL_TTS_MODEL=cosyvoice3
LOCAL_TTS_VOICE=default
```

## 8. GPU 现状

本次确认服务器有 10 张 `NVIDIA A100-SXM4-80GB`。

| GPU | 显存使用 MiB | 主要进程 |
| --- | ---: | --- |
| 0 | 34477 | LLM worker pid `1813602` |
| 1 | 33839 | LLM worker pid `1813602` |
| 2 | 34477 | LLM worker pid `1813603` |
| 3 | 33839 | LLM worker pid `1813603` |
| 4 | 34479 | LLM worker pid `1813604` |
| 5 | 33839 | LLM worker pid `1813604` |
| 6 | 67733 | `python` pid `1158458`，非核心链路，需确认 |
| 7 | 2135 | embedding worker pid `3747490` |
| 8 | 8090 | CosyVoice workers pid `367594` / `367595` |
| 9 | 8090 | CosyVoice workers pid `367596` / `367597` |

## 9. K8s 与 PostgreSQL

### K8s namespace

本项目相关 namespace：

```text
hnu-enrollment
```

本次确认资源：

```text
pod/hnu-enrollment-postgres-0
service/hnu-enrollment-postgres
statefulset.apps/hnu-enrollment-postgres
persistentvolumeclaim/hnu-enrollment-postgres-data
secret/hnu-enrollment-postgres-secret
```

PostgreSQL pod：

- namespace：`hnu-enrollment`
- pod：`hnu-enrollment-postgres-0`
- node：`train-2 / 10.10.200.13`
- image：`deploy.bocloud.k8s:40443/hnu-enrollment/pgvector:pg16`
- database：`hnu_enrollment`
- user：`postgres`
- password：来自 secret `hnu-enrollment-postgres-secret` 的 key `password`
- PVC：`hnu-enrollment-postgres-data`
- PV：`hnu-enrollment-postgres-pv`
- capacity：`30Gi`
- limits：CPU `2`，memory `2Gi`
- requests：CPU `250m`，memory `512Mi`

宿主机连接方式：

```bash
kubectl -n hnu-enrollment port-forward pod/hnu-enrollment-postgres-0 55432:5432
```

当前该 port-forward 已在服务器上运行，pid `3548470`。

### 当前数据库数据量

本次通过 `kubectl exec hnu-enrollment-postgres-0 -- psql ...` 验证：

| 表/视图 | count |
| --- | ---: |
| `admission_scores` | 3190 |
| `majors` | 212 |
| `provinces` | 31 |
| `knowledge_chunks` | 8636 |
| `faq_knowledge` | 136 |
| `conversation_messages` | 10005 |
| `admin_tickets` | 1 |
| `admin_settings` | 2 |
| `admin_audit_logs` | 2 |
| `admission_major_province_coverage` | 1105 |

数据类型：

- Excel 入库结构化数据：
  - `admission_scores`
  - `majors`
  - `provinces`
  - `admission_major_province_coverage` 物化视图
- PDF / document chunk：
  - `knowledge_chunks`
  - 文本和向量都保留，命中后文本进入上下文
  - 包含招生简章和各学院培养方案
- FAQ：
  - `faq_knowledge`
  - FAQ 也已向量化并写入 `knowledge_chunks`
- 对话/后台：
  - `conversation_messages`
  - `admin_tickets`
  - `admin_settings`
  - `admin_audit_logs`

物化视图脚本：

```text
/home/scm2002/Code/rust_enrollment/crates/importers/sql/admission_major_province_coverage.sql
```

本地常用命令：

```bash
cargo run -p importers --bin admission_coverage -- apply
cargo run -p importers --bin admission_coverage -- refresh
cargo run -p importers --bin admission_coverage -- verify
```

## 10. 应用环境变量摘要

服务器 Rust API 当前确认的关键环境：

```text
PORT=4000
DATABASE_URL=postgresql://***@localhost:55432/hnu_enrollment
LLM_PROVIDER=openai-compatible
LLM_MAX_CONCURRENT_REQUESTS=3
LLM_QUEUE_TIMEOUT_MS=20000
LOCAL_EMBEDDING_BASE_URL=http://127.0.0.1:8114/v1/embeddings
LOCAL_EMBEDDING_MODEL=bge-m3
LOCAL_TTS_STREAM_URL=http://127.0.0.1:50000/v1/audio/stream
LOCAL_TTS_SPEECH_URL=http://127.0.0.1:50000/v1/audio/speech
LOCAL_TTS_MODEL=cosyvoice3
LOCAL_TTS_VOICE=default
SERVER_TTS_MAX_CONCURRENT_SYNTH=4
SERVER_TTS_FIRST_QUEUE_TIMEOUT_MS=20000
SERVER_TTS_CONTINUATION_QUEUE_TIMEOUT_MS=120000
```

本地 `.env.example` 还包含：

```text
CHAT_MAX_CONCURRENT_REQUESTS=40
AGENT_TIMEOUT_SECS=75
CONVERSATION_HISTORY_WINDOW=40
CONVERSATION_TURN_LOCK_MAP_LIMIT=4096
MAJOR_CATALOG_CACHE_TTL_SECS=300
PROVINCE_MAJOR_LIST_DEFAULT_LIMIT=36
PROVINCE_MAJOR_LIST_EXPANDED_LIMIT=120
```

注意：

- `NEXT_PUBLIC_*` 会暴露在浏览器 JavaScript 中，不要放任何密钥。
- 生产后台管理 API token 应由香港 Nginx 注入，不应写入 admin 静态前端。

## 11. 前端与后台管理

### 学生/家长端

- 本地源码：`/home/scm2002/Code/rust_enrollment/apps/web`
- 服务器运行：`next start --hostname 127.0.0.1 --port 3000`
- 公网：`https://cpa.abolian.online/chat`
- 重点不要破坏：
  - `/api/v1/chat/stream`
  - `/api/v1/chat/voice`
  - server voice WebSocket
  - 新问题取消旧语音
  - agent 检索和短期记忆链路

### 后台管理端

- 本地源码：`/home/scm2002/Code/rust_enrollment/apps/admin`
- 香港静态部署目录：`/var/www/hnu-enrollment-admin`
- 公网：`https://admin.abolian.online/`
- 受 Cloudflare Access 保护。
- 最新部署过的 assets：
  - `index-DrQgHuqf.js`
  - `index-D9i87J4P.css`
- 当前未提交改动就是后台管理系统继续接真实数据和布局优化。

## 12. 音频缓存与日志

已有清理脚本：

```text
/home/scm2002/Code/rust_enrollment/scripts/prune-audio-cache.sh
```

默认保留最新 100 条音频/缓存文件，涉及目录包括：

```text
/home/t2_enroll_ai/rust_enrollment/tmp
/home/t2_enroll_ai/model-service-logs
```

后续建议：

- 将清理脚本放到服务器 cron 或 systemd timer。
- 不要无限保留历史语音音频。
- 对话历史可保留数据库，但后续需要管理端清理策略或归档策略。

## 13. 启停与验证命令

### 健康检查

内网服务器：

```bash
curl -fsS http://127.0.0.1:4000/api/v1/health
curl -I --max-time 8 http://127.0.0.1:3000/chat
```

公网：

```bash
curl -I --max-time 12 https://cpa.abolian.online/chat
curl -I --max-time 12 https://admin.abolian.online/
```

### 进程检查

```bash
ss -ltnp | grep -E ':(3000|4000|55432|18080|18081|18082|18083|8114|8116|50000|50001|50002|50003|50004)'
ps -eo pid,ppid,etime,cmd | grep -E 'api$|next-server|model_lb.py|llama-server|uvicorn|cosy|port-forward' | grep -v grep
```

### K8s 检查

```bash
kubectl get ns
kubectl -n hnu-enrollment get all,pvc,cm,secret -o wide
kubectl -n hnu-enrollment describe pod hnu-enrollment-postgres-0
```

### DB count 检查

宿主机当前没有 `psql`，可以直接进 pod 查询：

```bash
kubectl -n hnu-enrollment exec hnu-enrollment-postgres-0 -- \
  psql -U postgres -d hnu_enrollment -Atc 'select count(*) from knowledge_chunks;'
```

## 14. 10090 端口清理记录

用户指出截图中的 `10090` 可能可以删除。本次调查结果：

- 最初在项目服务器看到 `127.0.0.1:10090` 和 `[::1]:10090` 监听。
- `t2_enroll_ai` 不在 sudoers，无法在服务器侧直接用 `sudo ss` / `sudo lsof` 查 owner。
- 进一步在本地 WSL 进程表定位到来源：

```text
ssh -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 -o ServerAliveCountMax=3 \
  -N -R 10090:127.0.0.1:10090 t2_enroll_ai@10.10.200.13
```

- 该进程把本地 WSL 的 `10090` 反向暴露到项目服务器，属于临时代理/网络辅助链路，不是招生项目业务服务。
- 本次已停止本地进程 pid `142934`。
- 停止后本地 `ss -ltnp | grep ':10090'` 无输出。
- 停止后项目服务器 `ss -ltnp | grep ':10090'` 无输出。

后续如果再次出现 `10090`：

```bash
ps -eo pid,ppid,pgid,sid,etime,stat,cmd | grep -F -- '-R 10090:127.0.0.1:10090'
ss -ltnp | grep ':10090'
```

确认是临时反向隧道后再停止；不要误杀项目服务端口。

## 15. 已知风险与不要动的部分

不要随意改动：

- Rust API 的 chat / voice WebSocket 链路。
- CosyVoice 取消链路。
- `model-service-stack/model_lb.py` 到 4 个 CosyVoice worker 的分发逻辑。
- 3 个 LLM worker 的 `--parallel 1` 和 Rust 侧 `LLM_MAX_CONCURRENT_REQUESTS=3` 配套关系。
- K8s 里非 `hnu-enrollment` namespace 的资源。
- 香港服务器上现有 Cloudflare Tunnel 和 Nginx 路由。
- 后台管理端 Nginx 注入 admin token 的方式。

当前应优先做：

- 把后台管理 3 个未提交文件 review 后提交。
- 如果要同步服务器，先备份服务器未提交文件，再 rsync 或 git pull。
- 后续如继续优化后台管理系统，优先保持真实数据、无假数据兜底、可刷新、空状态清晰。
- 如继续优化语音并发，先压测并观察 first audio、segment gap、CosyVoice worker queue，不要凭感觉调大并发。

## 16. 参考文档

已有 runbook：

- `/home/scm2002/Code/rust_enrollment/docs/hong-kong-reverse-tunnel-runbook.md`
- `/home/scm2002/Code/rust_enrollment/docs/public-access-and-voice-ops-20260608.md`
- `/home/scm2002/Code/rust_enrollment/docs/admin-cloudflare-access-runbook.md`
- `/home/scm2002/Code/rust_enrollment/docs/cosyvoice-fp16-test-runbook-20260610.md`
- `/home/scm2002/Code/rust_enrollment/docs/admission-coverage-derived-table.md`

这些文档记录了香港中转、Cloudflare Access、语音服务恢复、CosyVoice fp16 测试、招生覆盖物化视图等历史操作。

## 17. 本次验证时间

- 本地时间：`2026-06-14 19:55 CST` 左右。
- 远端 `train-2` 时间：`2026-06-14 17:38 CST` 左右曾验证。
- 香港服务器时间：`2026-06-14 17:38 CST` 左右曾验证。

部分 SSH 命令曾因握手拥塞或 Tailscale 二次认证失败，本文中已经标注哪些是本次直接验证、哪些来自已有 runbook。
