# 招生智能体管理后台接入真实数据计划

## 目标

将原 `hashida` 后台管理前端迁入本项目，作为独立后台应用：

```text
apps/admin    # 招生智能体管理后台，Vite + React
apps/web      # 面向学生和家长的招生智能体前端，Next.js
apps/api      # Rust Axum API，继续承载 agent、语音、TTS、后台管理接口
```

后台管理系统应复用招生智能体项目的 Postgres 数据和 Rust 查询层，但不能破坏现有学生端业务，尤其不能影响：

- agent 路由、检索、短期记忆、LLM synthesis
- `/api/v1/chat`、`/chat/stream`、`/chat/voice`
- CosyVoice WebSocket 语音链路与取消链路
- 录取分数、FAQ、document chunk 检索

## 当前状态

`apps/admin` 目前是从 `/home/scm2002/Code/hashida` 迁入的前端复刻版。它现在仍然使用本地 mock 数据，登录也是演示态：

- mock 数据：`apps/admin/src/data/mock.ts`
- 页面集中在：`apps/admin/src/App.tsx`
- 登录：`admin / admin123` + `localStorage`

这一步只完成代码迁入，没有接入真实后端。

## 可直接复用的真实数据

来自现有招生项目 Postgres：

- `admission_scores`：2021-2025 分省、专业、科类、批次录取统计。
- `admission_major_province_coverage`：从录取统计派生的专业-省份覆盖物化视图。
- `majors`、`provinces`：专业、省份基础表。
- `faq_knowledge`：高频问答。
- `knowledge_chunks`：招生简章、培养方案、FAQ 向量化 chunk。
- `conversations`、`conversation_messages`：学生端对话历史。

这些数据适合支撑后台的：

- 录取分数查询与分布统计
- 专业、省份、批次、年份维度看板
- FAQ 管理与命中检索
- 文档知识库审计
- 对话记录审计

## 需要新增的管理数据

当前招生 agent 数据不足以支撑完整后台管理，需要新增独立管理表：

```text
admin_users
admin_sessions
admin_audit_logs
admin_settings
conversation_reviews
admin_tickets
analytics_events
evaluation_sessions
evaluation_results
knowledge_change_requests
```

建议原则：

- 后台管理数据与招生事实数据分层，避免污染 agent 查询表。
- 后台对 FAQ/知识库的写操作必须走审核和向量化流程。
- 所有管理员操作写 `admin_audit_logs`，便于上线追责。
- 对话审计不要修改原始 `conversation_messages`，审计结论写 `conversation_reviews`。

## Rust API 分层建议

在 `apps/api` 增加 `/api/v1/admin/*` 路由，但把业务实现拆到 crate，避免 `main.rs` 继续膨胀：

```text
crates/
  admin_domain/        # 后台 DTO、权限、分页、筛选类型
  admin_service/       # 后台业务编排、统计聚合、审核、工单
  db/                  # 继续保留底层 sqlx 查询，可新增 admin 查询方法
```

建议路由：

```text
POST   /api/v1/admin/auth/login
POST   /api/v1/admin/auth/logout
GET    /api/v1/admin/me

GET    /api/v1/admin/dashboard/summary
GET    /api/v1/admin/dashboard/trends
GET    /api/v1/admin/dashboard/provinces
GET    /api/v1/admin/dashboard/hot-questions

GET    /api/v1/admin/conversations
GET    /api/v1/admin/conversations/:id
PATCH  /api/v1/admin/conversations/:id/review

GET    /api/v1/admin/knowledge/faqs
POST   /api/v1/admin/knowledge/faqs
PATCH  /api/v1/admin/knowledge/faqs/:id
POST   /api/v1/admin/knowledge/faqs/:id/reembed

GET    /api/v1/admin/knowledge/chunks
GET    /api/v1/admin/knowledge/chunks/:id

GET    /api/v1/admin/tickets
POST   /api/v1/admin/tickets
PATCH  /api/v1/admin/tickets/:id

GET    /api/v1/admin/settings
PATCH  /api/v1/admin/settings
GET    /api/v1/admin/audit-logs
```

## 前端改造建议

`apps/admin` 后续应从单文件 mock 页面改为可迭代结构：

```text
apps/admin/src/
  api/                 # typed fetch client, envelope handling
  components/          # shell, cards, table, chart, modal, status
  pages/               # dashboard, insights, conversations, knowledge, tickets, settings
  types/               # Admin DTOs
  data/mock.ts         # 仅作为 dev fallback，最终逐步删除
```

前端 API 约定：

- `VITE_ADMIN_API_BASE_URL` 指向 Rust API。
- 所有请求走统一 `apiClient`，处理 `{ success, data, meta, error }` envelope。
- 页面先支持 loading/error/empty 状态，再替换 mock。
- 对话、知识库、工单等页面保留分页和筛选参数，避免一次性拉全量。

## 实施阶段

### Phase 1：迁入与隔离

- 将 `hashida` 前端迁入 `apps/admin`。
- 保持 mock 可运行。
- 不改学生端 `apps/web`。
- 不改 agent/voice/chat 业务。

### Phase 2：后台只读接口

- 新增 admin dashboard 只读接口。
- 新增 conversations 列表和详情接口。
- 新增 FAQ/chunk 只读接口。
- 前端逐页接入真实只读数据，mock 作为 fallback。

### Phase 3：管理写接口

- 管理员登录与 session。
- FAQ 新增/编辑/禁用。
- FAQ 变更触发 embedding 更新。
- 对话审核状态、人工介入标记。
- 工单新增和状态流转。
- 配置项保存。

### Phase 4：统计与观测

- 新增 `analytics_events` 或从 API trace 派生运营事件。
- 建立 dashboard 聚合查询或物化视图。
- 记录 admin 操作日志。
- 建立后台 API harness，覆盖权限、分页、写入、审计、FAQ re-embed。

## 安全边界

- 禁止继续使用演示账号和 localStorage 作为生产鉴权。
- 后台接口必须独立鉴权，不能暴露给学生端用户。
- 管理员密码使用 Argon2/bcrypt 哈希。
- session cookie 使用 `HttpOnly + SameSite + Secure`。
- 写操作全部记录审计日志。
- FAQ/知识库写入必须校验内容长度、来源、状态，避免错误内容进入 agent 上下文。

## 验收标准

- `apps/admin` 可以独立启动，不影响 `apps/web`。
- 学生端 chat/stream/voice 回归不受影响。
- 后台至少能读取真实 FAQ、conversation、admission 统计。
- 关键写操作有鉴权、审计、错误处理。
- FAQ 编辑后可以重新向量化，并被 agent 检索命中。
