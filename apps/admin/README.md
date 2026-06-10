# 哈师大招生智能体管理后台

独立 React + Vite + TypeScript 项目。该应用是招生智能体的后台管理前端，位于 `rust_enrollment/apps/admin`，与面向学生和家长的 `apps/web` 分开维护。

当前阶段仍使用本地 mock 数据，后续会通过 Rust Axum API 的 `/api/v1/admin/*` 接入真实 Postgres 数据。

## 启动

```bash
npm install
npm run dev
```

默认本地地址：`http://127.0.0.1:5173/`

演示账号：

```text
admin / admin123
```

注意：演示账号只用于本地开发，生产环境必须接入 Rust 后端管理员鉴权。

## 已实现页面

- `/login`
- `/`
- `/insights`
- `/special`
- `/evaluation-overview`
- `/evaluation`
- `/conversations`
- `/knowledge`
- `/tickets`
- `/settings`
- `/china-map`

## 检查

```bash
npm run typecheck
npm run lint
npm run build
```

## 集成计划

后台接入真实数据的架构和阶段计划见：

```text
docs/admin-backend-integration-plan.md
```
