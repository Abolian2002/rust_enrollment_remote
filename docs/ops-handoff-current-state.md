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
- 本地和服务器仓库的前端修改文件（`App.tsx`、`index.css`、`types/admin.ts`）以及 API 性能优化（内存 TTL 缓存 + try_join 并行化）已全部提交并合入 `main` 分支。
- 本地和服务器的提交历史与哈希已完全对齐（最新 commit 为 `7381af5`）。本地 `main` 追踪 `origin`（主库），服务器 `main` 追踪部署库 `rust_enrollment_remote.git`。
- `127.0.0.1:10090` 曾在内网项目服务器上监听；本地 macOS 的 SSH 反向隧道已被停止。

## 2. 仓库与路径

### 本地仓库

- 路径：`/Users/scm/code/rust_enrollment`
- 分支：`main`
- remotes：
  - `origin https://github.com/Abolian2002/rust_enrollment.git` (本地主开发库，后续本地修改推送到此处)
  - `enrollment_remote https://github.com/Abolian2002/rust_enrollment_remote.git` (本地部署分支的远程指针)
- 当前最新提交：
  - `7381af5 perf(admin): sync Cargo.lock with tokio db dependency`
  - `6209aac perf(admin): add 5-min TTL cache + parallel queries for admin endpoints`
  - `2c9c5e0 feat(admin): 更新后台登录页背景为夏天校园图，重命名xiatian.jpeg，添加运维交接文档`
  - `3c3e22a add public ticket submission flow`
- 本地工作区状态：Clean，无未提交文件。

### 内网项目服务器仓库

- SSH：`t2_enroll_ai@10.10.200.13`
- 密码：`qwer123456`
- hostname：`train-2`
- 项目路径：`/home/t2_enroll_ai/rust_enrollment`
- 远端仓库：`origin https://github.com/Abolian2002/rust_enrollment_remote.git` (服务器项目推拉至此处)
- 服务器仓库 HEAD 指针已与本地完美对齐，最新 commit 同样为 `7381af5`。
- 服务器工作区状态：Clean，无未提交文件。

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

### 内网自研模型项目服务器（Server 2）

- 地址：`10.10.200.13`
- 用户：`t2_enroll_ai`
- 密码：`qwer123456`
- 主机名：`train-2`
- 项目目录：`/home/t2_enroll_ai/rust_enrollment`
- 主要用途：
  - Rust API 后端
  - Next.js 学生/家长端前端
  - PostgreSQL 数据库端口转发
  - 我方自研微调大模型（7862 端口）及 CosyVoice/Embedding 服务

局域网内直连：

```bash
sshpass -p 'qwer123456' ssh t2_enroll_ai@10.10.200.13
```

### 内网竞品模型服务器（Server 1）

- 地址：`10.10.200.11`
- 用户：`t1_enroll_ai`
- 密码：`qwer123456`
- 主要用途：
  - 托管对方竞品模型服务（30080 端口）

局域网内直连：

```bash
sshpass -p 'qwer123456' ssh t1_enroll_ai@10.10.200.11
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
- 当前管理 key：`/Users/scm/.serect/hongkong.pem`
- 旧 key：`/Users/scm/.serect/abolian.pem`
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
ssh -i /Users/scm/.serect/hongkong.pem root@47.86.43.227
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

- 本地 token 文件路径：`/Users/scm/.serect/cloudflare_token`（若有）
- 文件权限：`600`
- 本次确认文件大小：`54` bytes
- 本次确认 SHA-256：`248537bb86dee3395b550f15faaf16bb932bfbd192552e1d358b824fedb054fd`
- 不要把 token 明文写入仓库或发到聊天里。
- 使用时建议：

```bash
export CLOUDFLARE_API_TOKEN="$(cat /Users/scm/.serect/cloudflare_token)"
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
| `10090` | `127.0.0.1` / `::1` | 已停止 | 原因是本地 macOS SSH 反向隧道已停止，非项目业务链路 |
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
/Users/scm/code/rust_enrollment/crates/importers/sql/admission_major_province_coverage.sql
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

- 本地源码：`/Users/scm/code/rust_enrollment/apps/web`
- 服务器运行：`next start --hostname 127.0.0.1 --port 3000`
- 公网：`https://cpa.abolian.online/chat`
- 重点不要破坏：
  - `/api/v1/chat/stream`
  - `/api/v1/chat/voice`
  - server voice WebSocket
  - 新问题取消旧语音
  - agent 检索和短期记忆链路

### 后台管理端

- 本地源码：`/Users/scm/code/rust_enrollment/apps/admin`
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
/Users/scm/code/rust_enrollment/scripts/prune-audio-cache.sh
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
- 进一步在本地 macOS 进程表定位到来源：

```text
ssh -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 -o ServerAliveCountMax=3 \
  -N -R 10090:127.0.0.1:10090 t2_enroll_ai@10.10.200.13
```

- 该进程把本地 macOS 的 `10090` 反向暴露到项目服务器，属于临时代理/网络辅助链路，不是招生项目业务服务。
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

- 持续监控后台管理端的接口加载表现，特别是缓存刷新行为及 API 日志。
- 后续如继续优化后台管理系统，优先保持真实数据、无假数据兜底、可刷新、空状态清晰。
- 如继续优化语音并发，先压测并观察 first audio、segment gap、CosyVoice worker queue，不要凭感觉调大并发。

## 16. 参考文档

已有 runbook：

- `/Users/scm/code/rust_enrollment/docs/hong-kong-reverse-tunnel-runbook.md`
- `/Users/scm/code/rust_enrollment/docs/public-access-and-voice-ops-20260608.md`
- `/Users/scm/code/rust_enrollment/docs/admin-cloudflare-access-runbook.md`
- `/Users/scm/code/rust_enrollment/docs/cosyvoice-fp16-test-runbook-20260610.md`
- `/Users/scm/code/rust_enrollment/docs/admission-coverage-derived-table.md`

这些文档记录了香港中转、Cloudflare Access、语音服务恢复、CosyVoice fp16 测试、招生覆盖物化视图等历史操作。

## 17. 后台白屏修复、免跳板机直连与微调模型上线记录（2026-06-18）

### 17.1 后台白屏故障修复
- **故障现象**：点击后台的“数据驾驶舱”（Dashboard）时，整个页面变为空白，应用崩溃。
- **故障排查**：在 [App.tsx](file:///Users/scm/code/rust_enrollment/apps/admin/src/App.tsx) 中定位到 `DashboardPage` 组件。其内部的 React 状态 Hook `const [timeRange, setTimeRange] = useState('近7天');` 被写在了 conditional returns（提前返回语句，如 `if (loading && !dashboard) return ...`）之后。这违反了 React 的 Hook 调用规则（Rules of Hooks），导致组件在数据异步加载完成并重绘时，调用 Hook 的顺序与数量发生变化而报错。
- **修复方案**：已将 `timeRange` 状态 Hook 的声明移至 `DashboardPage` 组件主体的最上方（在任何提前返回逻辑之前）。
- **验证与部署**：经本地 `npm run build` 成功通过编译；使用 Playwright 自动化脚本截图确认页面框架与加载态一切正常，无 React 错误。修复包已于 2026-06-16 编译打包并同步发布到香港服务器的静态根目录 `/var/www/hnu-enrollment-admin`，白屏问题彻底解决。

### 17.2 免跳板机直连项目服务器 (10.10.200.13)
- **网络背景**：本地 macOS 环境在连接局域网时可直连内网；若未连接局域网，亦可由本地运行的 SOCKS5 代理（Clash，端口 `10090`）打通内网。
- **直连方案**：利用 SSH 的 `ProxyCommand` 参数与 `nc` (netcat) 建立 TCP 转发隧道，通过本地的 SOCKS5 代理直连项目服务器，**可以免去 Tailscale 跳板机 (100.95.71.110) 的二次浏览器授权验证与中转**。
- **连接命令**：
  ```bash
  sshpass -p 'qwer123456' ssh \
    -o PreferredAuthentications=password \
    -o StrictHostKeyChecking=no \
    -o ProxyCommand="nc -X 5 -x 127.0.0.1:10090 %h %p" \
    t2_enroll_ai@10.10.200.13
  ```

### 17.3 下线三路基础模型实例并接入微调模型 (2026-06-18)
- **微调模型 API**：运行在项目服务器端口 `7862` (本地即 `http://127.0.0.1:7862/v1`)，基于 LLaMA-Factory 启动，使用 Qwen-35B-A3B 基础模型外挂 LoRA 适配器，支持 OpenAI 标准格式。
- **操作步骤**：
  1. **下线原三路基础模型 llama-server**：在项目服务器上杀死 `18081`、`18082`、`18083` 端口上运行的 `llama-server` 基础模型进程并清理 PID 文件：
     ```bash
     kill 1813602 1813603 1813604
     rm -f /home/t2_enroll_ai/model-service-run/qwen-llama-*.pid
     ```
  2. **切换 Rust API 配置指向**：修改项目服务器上的 `.run/api.env` 文件，将 API 请求基础 URL 指向微调模型端口：
     ```text
     OPENAI_COMPAT_BASE_URL=http://127.0.0.1:7862/v1
     ```
  3. **重启 API 服务**：
     ```bash
     kill $(cat /home/t2_enroll_ai/rust_enrollment/.run/api.pid)
     sleep 2
     cd /home/t2_enroll_ai/rust_enrollment && ./start-api.sh
     ```
     新启动的 API 进程 PID 为 `1308302`，成功监听 `4000` 端口，验证可用。

### 17.4 如何恢复/重新上线三路 llama-server 基础模型
1. **重新启动三路 llama-server 基础模型实例**：
   在项目服务器上执行如下启动命令（或直接执行 `/home/t2_enroll_ai/model-service-stack/start_high_concurrency.sh`）：
   ```bash
   # 启动实例 0
   cd /home/t2_enroll_ai/llama.cpp
   nohup env CUDA_VISIBLE_DEVICES="0,1" /home/t2_enroll_ai/llama.cpp/build/bin/llama-server \
     -m /home/t2_enroll_ai/models/Qwen3.6-35B-A3B-MTP-GGUF/BF16/Qwen3.6-35B-A3B-BF16-00001-of-00002.gguf \
     --alias qwen3.6-35b-a3b-bf16 --host 0.0.0.0 --port 18081 \
     -c 8192 --parallel 1 -ngl 999 --split-mode layer --jinja --reasoning off --reasoning-budget 0 \
     --api-key ragflow-local-qwen > /home/t2_enroll_ai/model-service-logs/qwen-llama-0.log 2>&1 &
   echo $! > /home/t2_enroll_ai/model-service-run/qwen-llama-0.pid

   # 启动实例 1
   nohup env CUDA_VISIBLE_DEVICES="2,3" /home/t2_enroll_ai/llama.cpp/build/bin/llama-server \
     -m /home/t2_enroll_ai/models/Qwen3.6-35B-A3B-MTP-GGUF/BF16/Qwen3.6-35B-A3B-BF16-00001-of-00002.gguf \
     --alias qwen3.6-35b-a3b-bf16 --host 0.0.0.0 --port 18082 \
     -c 8192 --parallel 1 -ngl 999 --split-mode layer --jinja --reasoning off --reasoning-budget 0 \
     --api-key ragflow-local-qwen > /home/t2_enroll_ai/model-service-logs/qwen-llama-1.log 2>&1 &
   echo $! > /home/t2_enroll_ai/model-service-run/qwen-llama-1.pid

   # 启动实例 2
   nohup env CUDA_VISIBLE_DEVICES="4,5" /home/t2_enroll_ai/llama.cpp/build/bin/llama-server \
     -m /home/t2_enroll_ai/models/Qwen3.6-35B-A3B-MTP-GGUF/BF16/Qwen3.6-35B-A3B-BF16-00001-of-00002.gguf \
     --alias qwen3.6-35b-a3b-bf16 --host 0.0.0.0 --port 18083 \
     -c 8192 --parallel 1 -ngl 999 --split-mode layer --jinja --reasoning off --reasoning-budget 0 \
     --api-key ragflow-local-qwen > /home/t2_enroll_ai/model-service-logs/qwen-llama-2.log 2>&1 &
   echo $! > /home/t2_enroll_ai/model-service-run/qwen-llama-2.pid
   ```
2. **将 Rust API 配置还原**：
   修改项目服务器的 `/home/t2_enroll_ai/rust_enrollment/.run/api.env` 文件，将 API 请求基础 URL 指回负载均衡端口：
   ```text
   OPENAI_COMPAT_BASE_URL=http://127.0.0.1:18080/v1
   ```
3. **重启 Rust API 后端**：
   ```bash
   kill $(cat /home/t2_enroll_ai/rust_enrollment/.run/api.pid)
   sleep 2
   cd /home/t2_enroll_ai/rust_enrollment && ./start-api.sh
   ```

## 18. 本次验证时间

- 本地时间：`2026-06-18 15:30 CST` 左右。
- 远端 `train-2` 时间：`2026-06-18 14:25 CST` 左右。
- 香港服务器时间：`2026-06-18 14:25 CST` 左右。

部分 SSH 命令曾因握手拥塞或 Tailscale 二次认证失败，本文中已经标注哪些是本次直接验证、哪些来自已有 runbook。

## 19. 双模型切换开关与本地语音服务调试记录（2026-06-21）

### 19.1 双模型切换设计与实现
- **需求目标**：实现前端首部模型切换开关（我方微调模型 vs 对方竞品模型），并在后端支持根据模型字段动态分发请求，同时保障本地测试环境下数字人语音合成（TTS）的连通性。
- **后端适配**：
  - 在 [domain/src/lib.rs](file:///Users/scm/code/rust_enrollment/crates/domain/src/lib.rs) 中修改 `ChatRequest` 结构，增加 `model: Option<String>` 可选字段，确保请求能携带所选模型。
  - 在 [admissions_agent/src/lib.rs](file:///Users/scm/code/rust_enrollment/crates/admissions_agent/src/lib.rs) 中动态判定 `req.model`。如果为 `"competitor"`，则构建并选用指向对方竞品模型（Server 1 30080 端口）的 `llm_theirs` 客户端，否则默认使用我方微调模型（Server 2 7862 端口）的 `llm` 客户端。
- **前端页面与组件**：
  - 在 [api-client.ts](file:///Users/scm/code/rust_enrollment/apps/web/lib/api-client.ts) 中扩展 `ChatRequest` 接口。
  - 在 [use-chat-session.ts](file:///Users/scm/code/rust_enrollment/apps/web/components/use-chat-session.ts) 中引入 `model` 状态并伴随请求发送。
  - 修改 [page.tsx](file:///Users/scm/code/rust_enrollment/apps/web/app/chat/page.tsx)，在 Header 部分引入了玻璃质感的模型切换控件，并带有动态呼吸指示灯（绿色代表我方自研模型，红色代表对方竞品模型）。

### 19.2 本地运行环境搭建与故障排查
- **SSH 端口转发隧道**：
  - 通过 `start_tunnels.py` 在后台建立两条免跳转 SSH 直连隧道：
    - **Tunnel 1 (Server 2)**：本地 `7862` -> 远程模型服务，本地 `8114` -> 远程向量检索服务，本地 `50000` -> 远程 CosyVoice 语音服务。
    - **Tunnel 2 (Server 1)**：本地 `30080` -> 远程竞品模型服务。
- **数据库连接配置**：
  - 修改本地 `DATABASE_URL`，将数据库连接用户名由 `postgres` 调整为 macOS 的系统角色 `scm`（无密码连接），解决了最初 `CHAT_ERROR` 导致的数据库连接重置故障。
- **语音服务不可用（BAD_GATEWAY）故障解决**：
  - **故障现象**：在浏览器测试时，文字问答正常但系统抛出“语音服务暂时不可用，请稍后再试”错误。
  - **排查定位**：通过直接 `curl` 本地 `4000` 端口的 `/api/v1/tts/speech` 及 `/api/v1/tts/stream` 确认后端服务本身可用（能返回音频二进制流）。定位到问题为：本地运行的 Rust API 进程（PID 49799）和 Next.js dev 进程（PID 49832）启动时间较早，并未加载到后面配置的最新环境变量。前端因读取不到 `NEXT_PUBLIC_TTS_TRANSPORT=local-stream` 默认回退使用 DashScope API，但本地又无 `DASHSCOPE_API_KEY`，进而导致连接失败。
  - **修复方案**：杀死全部陈旧的后台 API 进程和 Next.js 进程，以最新环境配置重新在后台拉起 `cargo run --bin api` 以及 `npm run dev`。服务重载后，本地数字人语音合成与播放功能彻底恢复正常。
- **测试通过**：
  - 本地运行 `npm run test` 与 `cargo test` 全部测试百分之百通过。

### 19.3 数字人语音播报（TTS）两大核心故障定位与修复（2026-06-21）
- **故障 1：语音卡死无声音（Session Superseded 竞态）**：
  - **故障现象**：数字人头像提示“语音正在连接/播报中”，服务端语音合成完成并发送，但前端浏览器没有输出 `pushPCM: received` 日志，无声音发出。
  - **根本原因**：前端点击发送时，在同一手势调用栈中先执行 `void voice.prepare()`，紧接着同步执行了 `void voice.interrupt()`。`interrupt` 导致全局会话计数器 `sessionRunId.current` 同步递增，导致异步完成的 `prepare` 在检测会话有效性时，因 ID 不匹配而触发 `"TTS session superseded"` 错误，刚建立的播放器被立刻销毁。
  - **修复方案**：在 [page.tsx](file:///Users/scm/code/rust_enrollment/apps/web/app/chat/page.tsx) 中调整调用顺序，先执行 `voice.interrupt()` 打断清理旧播放，再同步启动 `voice.prepare()` 开启新会话，确保计数器状态对齐。
- **故障 2：香港公网 HTTP 访问显示“语音暂不可用”**：
  - **根本原因**：`server-voice` 模式需使用 `AudioWorklet` 处理 PCM 裸流。出于安全性设计，现代浏览器规定 `AudioWorklet` 仅能在安全上下文（HTTPS 或 localhost）下启用。在公网 HTTP (`http://cpa.abolian.online/chat`) 环境下，浏览器禁用该组件，从而导致数字人左侧报“语音暂不可用”。
  - **修复方案**：在 `ChatPage` 中加入自动 HTTPS 重定向策略，若检测到在非 localhost 生产环境使用 HTTP 协议访问，自动将协议升级至 HTTPS（因配置了 Cloudflare 的 HTTPS 代理，此过程对用户无感），彻底解决非安全上下文禁用 `AudioWorklet` 的限制。该方案已在服务器 2 上重新部署验证成功。

## 20. 单模型模式强制及 Qwen3.5 CoT 思考过程屏蔽记录（2026-06-25）

### 20.1 强制单模型模式
- **前端调整**：从首部彻底删除了我方自研 vs 对方竞品模型切换控件，并将 `selectedModel` 强制固定为 `"ours"`，确保所有前端请求固定指向我方模型。
- **后端简化**：修改 `crates/admissions_agent/src/lib.rs` 的 `chat` 及 `chat_stream_with_deltas` 函数，废弃对请求参数中 `model` 的判断，强制使用 `self.llm` 客户端。

### 20.2 Qwen3.5 CoT 思考过程屏蔽
- **Jinja 模版传参修复**：Qwen3.5 采用内置 Jinja 模版渲染 prompt。若未显式传参 `enable_thinking: false`，模型默认会输出思考过程。我们更新了 `crates/llm/src/lib.rs` 的 API 荷载结构，在请求体中增加了 vLLM 所要求的 `chat_template_kwargs`：
  ```json
  "chat_template_kwargs": {
      "enable_thinking": false
  }
  ```
  以此在模型模版渲染层关闭思考输出，避开了因 1600 tokens 长度限制中途截断而暴露思考块的故障。
- **Prompt 强化**：在合成应答 Prompt 中追加了关于屏蔽 `<think>` 和 `Thinking Process:` 的重要规则约束。
- **历史数据清洗**：由于模型具备 In-Context Learning (上下文学习) 特性，在多轮对话中会模仿历史中已有的推理过程结构。我们在 remote 数据库中执行了清洗语句，将 `conversation_messages` 中残留的历史 "Thinking Process:" 回答内容全部重置。

### 20.3 vLLM 服务运行环境
Qwen 3.5 MoE-122B-A10B 运行在 Server 2 的虚拟环境中，具体信息如下：
- **运行端口**：`7868`
- **运行进程 PID**：`3721828` (ll3 conda 环境)
- **显存与 GPU 拓扑**：使用 `--tensor-parallel-size 4` 张卡并行，每张卡占用约 34GB 显存。
- **服务启动命令**：
  ```bash
  /home/t2_enroll_ai/miniconda3/envs/ll3/bin/python -m vllm.entrypoints.openai.api_server \
    --model /home/t2_enroll_ai/llmws/t4/LLaMA-Factory/qwen122ba10b \
    --served-model-name qwen \
    --tensor-parallel-size 4 \
    --port 7868 \
    --trust-remote-code \
    --limit-mm-per-prompt '{"image":16,"video":4}'
  ```

### 20.4 ll3 Conda 环境依赖包

Server 2 上 vLLM 运行在 Conda 环境 `ll3` 中（Python 解释器路径：`/home/t2_enroll_ai/miniconda3/envs/ll3/bin/python`）。

> 注意：该环境的 `pip` shebang 已损坏，列出包需使用 `python -m pip list`。

#### 核心框架与推理引擎

| 包 | 版本 | 说明 |
| --- | --- | --- |
| `vllm` | `0.17.0` | 推理服务主引擎 |
| `torch` | `2.10.0` | PyTorch |
| `torchaudio` | `2.10.0` | |
| `torchvision` | `0.25.0` | |
| `transformers` | `5.6.0` | HuggingFace Transformers |
| `tokenizers` | `0.22.2` | 分词器 |
| `accelerate` | `1.11.0` | 分布式加速 |
| `safetensors` | `0.8.0` | 模型权重格式 |
| `xformers` | `0.0.29.post2` | 高效注意力算子 |
| `triton` | `3.6.0` | Triton JIT 编译器 |
| `flashinfer-python` | `0.6.4` | FlashInfer 注意力后端 |
| `xgrammar` | `0.1.29` | 结构化生成 |

#### CUDA / NVIDIA 依赖

| 包 | 版本 |
| --- | --- |
| `nvidia-cublas-cu12` | `12.8.4.1` |
| `nvidia-cuda-runtime-cu12` | `12.8.90` |
| `nvidia-cudnn-cu12` | `9.10.2.21` |
| `nvidia-nccl-cu12` | `2.27.5` |
| `nvidia-cufft-cu12` | `11.3.3.83` |
| `nvidia-cusparse-cu12` | `12.5.8.93` |
| `nvidia-cusparselt-cu12` | `0.7.1` |
| `nvidia-nvjitlink-cu12` | `12.8.93` |
| `nvidia-nvshmem-cu12` | `3.4.5` |
| `cuda-bindings` | `12.9.4` |
| `cupy-cuda12x` | `13.6.0` |

#### 训练 / 微调相关

| 包 | 版本 | 说明 |
| --- | --- | --- |
| `llamafactory` | `0.9.6.dev0` | LLaMA-Factory（editable install） |
| `peft` | `0.18.1` | LoRA / 参数高效微调 |
| `trl` | `0.24.0` | 强化学习训练 |
| `datasets` | `4.0.0` | HuggingFace Datasets |
| `sentencepiece` | `0.2.1` | 分词 |
| `tiktoken` | `0.13.0` | OpenAI 分词 |

#### API / Web 服务

| 包 | 版本 |
| --- | --- |
| `openai` | `2.24.0` |
| `fastapi` | `0.137.0` |
| `uvicorn` | `0.49.0` |
| `gradio` | `5.50.0` |
| `starlette` | `1.3.1` |
| `huggingface_hub` | `1.19.0` |

#### 其他关键包

| 包 | 版本 |
| --- | --- |
| `numpy` | `2.2.6` |
| `scipy` | `1.17.1` |
| `pandas` | `2.3.3` |
| `scikit-learn` | `1.9.0` |
| `ray` | `2.55.1` |
| `compressed-tensors` | `0.13.0` |
| `gguf` | `0.19.0` |
| `supervisor` | `4.3.0` |
| `Jinja2` | `3.1.6` |
| `protobuf` | `6.33.6` |
| `PyYAML` | `6.0.3` |

> `flash-attn` 未安装；vLLM 0.17.0 使用 `flashinfer-python 0.6.4` 作为注意力后端。
