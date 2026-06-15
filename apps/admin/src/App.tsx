import { useCallback, useEffect, useState, type FormEvent, type ReactNode } from 'react';
import { BrowserRouter, Link, Navigate, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import ReactECharts from 'echarts-for-react';
import * as echarts from 'echarts';
import chinaMap from './data/china-map.json';
import {
  AlertCircle,
  BarChart3,
  Bell,
  BookOpen,
  CheckCircle2,
  ChevronDown,
  Clock3,
  Download,
  Eye,
  FileText,
  Filter,
  Globe2,
  LayoutDashboard,
  Mail,
  MapPinned,
  Menu,
  MessageSquare,
  Plus,
  RefreshCw,
  Save,
  Search,
  Settings,
  Target,
  TrendingUp,
  Upload,
  Users,
  X,
  type LucideIcon,
} from 'lucide-react';
import {
  fetchAdminAdmissionsAnalytics,
  fetchAdminBigScreen,
  fetchAdminConversationDetail,
  fetchAdminConversations,
  fetchAdminDashboard,
  fetchAdminFaqs,
  fetchAdminInsights,
  fetchAdminKnowledgeChunks,
  fetchAdminKnowledgeCoverage,
  fetchAdminSpecial,
  fetchAdminAuditLogs,
  fetchAdminSettings,
  fetchAdminTickets,
  createAdminFaq,
  createAdminFeedback,
  updateAdminFaq,
  updateAdminSettings,
  updateAdminTicket,
} from './api/admin';
import type {
  AdminAuditLogItem,
  AdminAdmissionsAnalyticsSnapshot,
  AdminBigScreenSnapshot,
  AdminConversationDetail,
  AdminConversationListItem,
  AdminDashboardSnapshot,
  AdminFeedbackType,
  AdminInsightsSnapshot,
  AdminKnowledgeChunkItem,
  AdminKnowledgeCoverageSnapshot,
  AdminSettings,
  AdminSpecialSnapshot,
  AdminTicketItem,
  KnowledgeItem,
  Stat,
} from './types/admin';

echarts.registerMap('china', chinaMap as never);

const navItems = [
  { label: '全国招生咨询态势', path: '/china-map', icon: Globe2, external: true },
  { label: '数据驾驶舱', path: '/', icon: LayoutDashboard },
  { label: '热点问题分析', path: '/insights', icon: TrendingUp },
  { label: '专项招生看板', path: '/special', icon: BarChart3 },
  { label: '测评数据总览', path: '/evaluation-overview', icon: TrendingUp },
  { label: '测评明细', path: '/evaluation', icon: FileText },
  { label: '对话记录审计', path: '/conversations', icon: MessageSquare },
  { label: '知识库管理', path: '/knowledge', icon: BookOpen },
  { label: '留言工单', path: '/tickets', icon: Mail },
  { label: '系统配置', path: '/settings', icon: Settings },
];

const toneIconClass: Record<NonNullable<Stat['tone']>, string> = {
  blue: 'tone-blue',
  green: 'tone-green',
  cyan: 'tone-cyan',
  amber: 'tone-amber',
  red: 'tone-red',
  purple: 'tone-purple',
};

const monthFilters = ['近7天', '近30天', '全年'];

function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/login" element={<LoginPage />} />
        <Route path="/china-map" element={<ProtectedRoute><BigScreenPage /></ProtectedRoute>} />
        <Route path="/*" element={<ProtectedRoute><AdminRoutes /></ProtectedRoute>} />
      </Routes>
    </BrowserRouter>
  );
}

function ProtectedRoute({ children }: { children: ReactNode }) {
  const authed = localStorage.getItem('hashida-auth') === 'ok';
  return authed ? children : <Navigate to="/login" replace />;
}

function AdminRoutes() {
  return (
    <AdminLayout>
      <Routes>
        <Route path="/" element={<DashboardPage />} />
        <Route path="/insights" element={<InsightsPage />} />
        <Route path="/special" element={<SpecialPage />} />
        <Route path="/evaluation-overview" element={<EvaluationOverviewPage />} />
        <Route path="/evaluation" element={<EvaluationPage />} />
        <Route path="/conversations" element={<ConversationsPage />} />
        <Route path="/knowledge" element={<KnowledgePage />} />
        <Route path="/tickets" element={<TicketsPage />} />
        <Route path="/settings" element={<SettingsPage />} />
      </Routes>
    </AdminLayout>
  );
}

function LoginPage() {
  const navigate = useNavigate();
  const [username, setUsername] = useState('admin');
  const [password, setPassword] = useState('admin123');
  const [visible, setVisible] = useState(false);
  const [error, setError] = useState('');

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (username === 'admin' && password === 'admin123') {
      localStorage.setItem('hashida-auth', 'ok');
      navigate('/', { replace: true });
      return;
    }
    setError('账号或密码不正确，请使用演示账号 admin / admin123');
  }

  return (
    <main className="login-page">
      <div className="login-scrim" />
      <section className="login-panel">
        <img src="/assets/hsul-logo.png" alt="哈尔滨师范大学校徽" className="login-logo" />
        <h1>哈尔滨师范大学</h1>
        <p className="login-subtitle">招生智能体管理后台</p>
        <form className="login-card" onSubmit={submit}>
          <label>
            <span>用户名</span>
            <input value={username} onChange={(event) => setUsername(event.target.value)} />
          </label>
          <label>
            <span>密码</span>
            <div className="password-field">
              <input type={visible ? 'text' : 'password'} value={password} onChange={(event) => setPassword(event.target.value)} />
              <button type="button" onClick={() => setVisible((value) => !value)}>
                <Eye size={18} />
              </button>
            </div>
          </label>
          {error ? <div className="form-error">{error}</div> : null}
          <button className="login-button" type="submit">登 录</button>
          <div className="demo-account">演示账号：admin / admin123</div>
        </form>
        <p className="copyright">版权所有 哈尔滨师范大学招生办公室</p>
      </section>
    </main>
  );
}

function AdminLayout({ children }: { children: ReactNode }) {
  const [collapsed, setCollapsed] = useState(false);
  const [toast, setToast] = useState('');
  const location = useLocation();

  function notify(text: string) {
    setToast(text);
    window.setTimeout(() => setToast(''), 1800);
  }

  return (
    <div className="admin-shell">
      <aside className={`sidebar ${collapsed ? 'collapsed' : ''}`}>
        <div className="brand">
          <img src="/assets/hsul-logo.png" alt="校徽" />
          {!collapsed ? <div><strong>哈师大招生智能体</strong><span>管理后台</span></div> : null}
        </div>
        <nav className="side-nav">
          {navItems.map((item) => {
            const Icon = item.icon;
            const active = item.path === '/' ? location.pathname === '/' : location.pathname.startsWith(item.path);
            return (
              <Link className={`side-link ${active ? 'active' : ''}`} key={item.path} to={item.path}>
                <Icon size={20} />
                {!collapsed ? <span>{item.label}</span> : null}
                {!collapsed && item.external ? <MapPinned size={13} className="side-extra" /> : null}
              </Link>
            );
          })}
        </nav>
        <button className="collapse-button" type="button" onClick={() => setCollapsed((value) => !value)}>
          <Menu size={18} />
          {!collapsed ? <span>收起菜单</span> : null}
        </button>
      </aside>
      <div className="workspace">
        <header className="topbar">
          <div />
          <div className="top-actions">
            <button type="button" className="icon-button" onClick={() => notify('暂无新的系统通知')}>
              <Bell size={18} />
              <span className="dot" />
            </button>
            <button type="button" className="admin-chip" onClick={() => notify('当前账号：招生办管理员')}>
              <span>A</span>
              <div><strong>admin</strong><small>招生办管理员</small></div>
            </button>
          </div>
        </header>
        <main className="content">{children}</main>
      </div>
      {toast ? <Toast text={toast} /> : null}
    </div>
  );
}

function Toast({ text }: { text: string }) {
  return <div className="toast"><CheckCircle2 size={16} />{text}</div>;
}

function PageTitle({ title, subtitle }: { title?: string; subtitle?: string }) {
  if (!title) return null;
  return (
    <div className="page-title">
      <h1>{title}</h1>
      {subtitle ? <p>{subtitle}</p> : null}
    </div>
  );
}

function StatGrid({ stats }: { stats: Stat[] }) {
  return (
    <div className="stat-grid">
      {stats.map((stat) => (
        <div className="stat-card" key={stat.label}>
          <div className={`stat-icon ${toneIconClass[stat.tone ?? 'blue']}`}>
            {stat.tone === 'red' ? <AlertCircle size={24} /> : stat.tone === 'amber' ? <TrendingUp size={24} /> : <Users size={24} />}
          </div>
          <span>{stat.label}</span>
          <strong>{stat.value}</strong>
          {stat.delta ? <small className={stat.delta.startsWith('-') ? 'down' : 'up'}>↗ {stat.delta} 较上周期</small> : <small>&nbsp;</small>}
        </div>
      ))}
    </div>
  );
}

function Card({ title, children, action, className = '' }: { title?: string; children: ReactNode; action?: ReactNode; className?: string }) {
  return (
    <section className={`card ${className}`}>
      {title ? (
        <div className="card-head">
          <h2>{title}</h2>
          {action}
        </div>
      ) : null}
      {children}
    </section>
  );
}

function Toolbar({ children }: { children: ReactNode }) {
  return <div className="toolbar">{children}</div>;
}

function SelectLike({ children }: { children: ReactNode }) {
  return <button type="button" className="select-like">{children}<ChevronDown size={16} /></button>;
}

function SearchBox({ value, onChange, placeholder }: { value: string; onChange: (value: string) => void; placeholder: string }) {
  return (
    <label className="search-box">
      <Search size={17} />
      <input placeholder={placeholder} value={value} onChange={(event) => onChange(event.target.value)} />
    </label>
  );
}

function PrimaryButton({ children, onClick, ghost = false }: { children: ReactNode; onClick?: () => void; ghost?: boolean }) {
  return <button type="button" className={ghost ? 'ghost-button' : 'primary-button'} onClick={onClick}>{children}</button>;
}

function StatusBadge({ value }: { value: string }) {
  const normalized = value.includes('待') ? 'pending' : value.includes('中') ? 'working' : value.includes('误') || value.includes('弃') ? 'danger' : value.includes('禁') || value.includes('关闭') ? 'muted' : 'ok';
  return <span className={`status ${normalized}`}>{value}</span>;
}

function RefreshAction({ loading, onClick }: { loading?: boolean; onClick: () => void }) {
  return (
    <button type="button" className="refresh-action" onClick={onClick} disabled={loading}>
      <RefreshCw size={16} className={loading ? 'spinning' : ''} />
      {loading ? '刷新中' : '刷新'}
    </button>
  );
}

function LoadError({ message, onRetry }: { message: string; onRetry?: () => void }) {
  return (
    <div className="warning-box">
      <AlertCircle size={18} />
      <span>真实数据读取失败：{message}</span>
      {onRetry ? <button type="button" className="inline-retry" onClick={onRetry}>重试</button> : null}
    </div>
  );
}

function EmptyState({ children = '暂无真实数据。', compact = false }: { children?: ReactNode; compact?: boolean }) {
  return <div className={`empty-state ${compact ? 'compact' : ''}`}>{children}</div>;
}

function Modal({ title, children, onClose }: { title: string; children: ReactNode; onClose: () => void }) {
  return (
    <div className="modal-layer" role="dialog" aria-modal="true">
      <div className="modal">
        <div className="modal-head">
          <h2>{title}</h2>
          <button type="button" onClick={onClose}><X size={18} /></button>
        </div>
        <div className="modal-body">{children}</div>
      </div>
    </div>
  );
}

function DashboardPage() {
  const [dashboard, setDashboard] = useState<AdminDashboardSnapshot | null>(null);
  const [loadError, setLoadError] = useState('');
  const [loading, setLoading] = useState(true);

  const loadDashboard = useCallback(() => {
    setLoading(true);
    fetchAdminDashboard()
      .then((data) => {
        setDashboard(data);
        setLoadError('');
      })
      .catch((error: Error) => {
        setDashboard(null);
        setLoadError(error.message);
      })
      .finally(() => {
        setLoading(false);
      });
  }, []);

  useEffect(() => {
    loadDashboard();
  }, [loadDashboard]);

  if (loading && !dashboard) {
    return <EmptyState>正在读取真实驾驶舱数据...</EmptyState>;
  }

  if (loadError && !dashboard) {
    return <LoadError message={loadError} onRetry={loadDashboard} />;
  }

  if (!dashboard) {
    return <EmptyState>暂无真实驾驶舱数据。</EmptyState>;
  }

  const trendLabels = dashboard.trendDays.length === dashboard.trendValues.length
    ? dashboard.trendDays
    : dashboard.trendValues.map((_, index) => `第${index + 1}天`);

  return (
    <>
      <div className="page-meta">
        <SelectLike>近7天</SelectLike>
        <span><Clock3 size={16} /> 数据更新时间：{dashboard.updatedAt}</span>
        <RefreshAction loading={loading} onClick={loadDashboard} />
      </div>
      {loadError ? <LoadError message={loadError} onRetry={loadDashboard} /> : null}
      {dashboard.stats.length ? <StatGrid stats={dashboard.stats} /> : <EmptyState compact>暂无统计指标。</EmptyState>}
      <div className="grid-two">
        <Card title="咨询量趋势" action={<span className="soft-pill">真实咨询日志聚合</span>}>
          {dashboard.trendValues.length ? <Chart option={lineOption(trendLabels, dashboard.trendValues, '#2161ff')} height={290} /> : <EmptyState compact>暂无趋势数据。</EmptyState>}
        </Card>
        <Card title="24小时咨询时段分布">
          {dashboard.hourlyValues.length ? <Chart option={barOption(Array.from({ length: dashboard.hourlyValues.length }, (_, index) => `${index.toString().padStart(2, '0')}`), dashboard.hourlyValues, '#34c8c2')} height={290} /> : <EmptyState compact>暂无时段分布数据。</EmptyState>}
        </Card>
      </div>
      <div className="grid-two">
        <Card title="近期热点问题 TOP5" action={<a className="linkish" href="/insights">查看全部</a>}>
          {dashboard.hotQuestions.length ? (
            <ol className="rank-list">
              {dashboard.hotQuestions.slice(0, 5).map(([question, count], index) => <li key={question}><b>{index + 1}</b><span>{question}</span><em>{count}</em></li>)}
            </ol>
          ) : (
            <EmptyState compact>暂无热点问题数据。</EmptyState>
          )}
        </Card>
        <Card title="预警信息与快捷入口">
          <EmptyState compact>预警规则接口尚未接入，暂不展示估算数据。</EmptyState>
          <div className="quick-grid">
            {['高频问题分析', '工单待办', '知识库更新', '待审核对话'].map((item) => <button type="button" key={item}>{item}</button>)}
          </div>
        </Card>
      </div>
    </>
  );
}

function InsightsPage() {
  const [month, setMonth] = useState(monthFilters[1]);
  const [query, setQuery] = useState('');
  const [insights, setInsights] = useState<AdminInsightsSnapshot | null>(null);
  const [loadError, setLoadError] = useState('');
  const [loading, setLoading] = useState(true);

  const loadInsights = useCallback(() => {
    setLoading(true);
    fetchAdminInsights()
      .then((data) => {
        setInsights(data);
        setLoadError('');
      })
      .catch((error: Error) => {
        setInsights(null);
        setLoadError(error.message);
      })
      .finally(() => {
        setLoading(false);
      });
  }, []);

  useEffect(() => {
    loadInsights();
  }, [loadInsights]);

  const topRows = insights?.topQuestions.map((item) => [item.question, item.category, String(item.count), item.share] as [string, string, string, string]) ?? [];
  const filtered = topRows.filter((row) => row[0].includes(query) || row[1].includes(query));
  const wordCloud = insights?.wordCloud.map((item) => item.name) ?? [];

  return (
    <>
      <PageTitle title="招生咨询热点洞察" subtitle={insights ? `真实数据更新时间：${insights.updatedAt}` : undefined} />
      <Card>
        <div className="month-tabs">
          {monthFilters.map((item) => <button className={item === month ? 'active' : ''} type="button" key={item} onClick={() => setMonth(item)}>{item}</button>)}
          <span className="trend-alert">{insights ? '真实咨询日志聚合' : '等待真实数据'}</span>
          <RefreshAction loading={loading} onClick={loadInsights} />
        </div>
        {loadError ? <LoadError message={loadError} onRetry={loadInsights} /> : null}
        {loading && !insights ? <EmptyState compact>正在读取真实热点数据...</EmptyState> : null}
        {insights ? (
          <div className="insight-hero">
            <MiniMetric icon={Users} label={insights.stats[0]?.label ?? '咨询用户数'} value={insights.stats[0]?.value ?? '0'} />
            <MiniMetric icon={MessageSquare} label={insights.stats[1]?.label ?? '咨询问答数'} value={insights.stats[1]?.value ?? '0'} />
            <MiniMetric icon={Target} label={insights.stats[2]?.label ?? '高意向留资量'} value={insights.stats[2]?.value ?? '0'} />
            <p><b>数据特征：</b>{insights.summary || '暂无摘要。'}</p>
            <p><b>重点关注：</b>后台按分数位次、录取规则、专业介绍、专项政策、校园生活等通用类别归集用户问题，用于辅助招生办及时发现咨询高峰和知识库缺口。</p>
            <div className="tag-row">{wordCloud.slice(0, 4).map((word) => <span key={word}>{word}</span>)}</div>
          </div>
        ) : null}
      </Card>
      <Toolbar>
        <SelectLike>{month}</SelectLike>
        <SelectLike>全部分类</SelectLike>
        <SearchBox value={query} onChange={setQuery} placeholder="搜索问题关键词..." />
        <PrimaryButton ghost><Download size={16} />导出Excel</PrimaryButton>
      </Toolbar>
      <div className="grid-two">
        <Card title="咨询内容分类统计">{insights?.categoryStats.length ? <Chart option={pieOption(insights.categoryStats)} height={280} /> : <EmptyState compact>暂无分类统计。</EmptyState>}</Card>
        <Card title="生源地域热度分析">{insights?.provinceBars.length ? <Chart option={horizontalBarOption(insights.provinceBars)} height={280} /> : <EmptyState compact>暂无地域热度数据。</EmptyState>}</Card>
      </div>
      <Card title="高频问题 TOP20 榜单">
        {filtered.length ? <DataTable headers={['排名', '问题内容', '分类', '提问次数', '占比']} rows={filtered.map((row, index) => [index + 1, row[0], row[1], row[2], row[3]])} /> : <EmptyState compact>暂无匹配的真实高频问题。</EmptyState>}
      </Card>
      <Card title="用户关注点词云">
        {wordCloud.length ? <div className="word-cloud">{wordCloud.map((word, index) => <span style={{ fontSize: `${14 + (index % 6) * 4}px` }} key={word}>{word}</span>)}</div> : <EmptyState compact>暂无词云数据。</EmptyState>}
      </Card>
    </>
  );
}

function SpecialPage() {
  const [special, setSpecial] = useState<AdminSpecialSnapshot | null>(null);
  const [admissions, setAdmissions] = useState<AdminAdmissionsAnalyticsSnapshot | null>(null);
  const [loadError, setLoadError] = useState('');
  const [loading, setLoading] = useState(true);

  const loadSpecial = useCallback(() => {
    setLoading(true);
    Promise.all([fetchAdminSpecial(), fetchAdminAdmissionsAnalytics()])
      .then(([specialData, admissionsData]) => {
        setSpecial(specialData);
        setAdmissions(admissionsData);
        setLoadError('');
      })
      .catch((error: Error) => {
        setSpecial(null);
        setAdmissions(null);
        setLoadError(error.message);
      })
      .finally(() => {
        setLoading(false);
      });
  }, []);

  useEffect(() => {
    loadSpecial();
  }, [loadSpecial]);

  return (
    <>
      <div className="page-meta">
        <span>{special ? `专项数据更新时间：${special.updatedAt}` : '专项招生看板'}</span>
        <RefreshAction loading={loading} onClick={loadSpecial} />
      </div>
      {loadError ? <LoadError message={loadError} onRetry={loadSpecial} /> : null}
      {loading && !special ? <EmptyState>正在读取真实专项数据...</EmptyState> : null}
      {special?.stats.length ? <StatGrid stats={special.stats} /> : !loading ? <EmptyState compact>暂无专项统计指标。</EmptyState> : null}
      <div className="grid-two">
        <Card title="师范类 vs 非师范类咨询对比">{special?.normalVsNonNormal.length ? <Chart option={pieOption(special.normalVsNonNormal)} height={270} /> : <EmptyState compact>暂无师范类对比数据。</EmptyState>}</Card>
        <Card title="专项与政策咨询量">{special?.specialPlans.length ? <Chart option={barOption(special.specialPlans.map((item) => item[0] as string), special.specialPlans.map((item) => item[1] as number), '#3478f6')} height={270} /> : <EmptyState compact>暂无专项政策咨询数据。</EmptyState>}</Card>
      </div>
      <div className="grid-two">
        <Card title="各专业考生关注度 TOP10">{special?.majorAttention.length ? <Chart option={horizontalBarOption(special.majorAttention)} height={340} /> : <EmptyState compact>暂无专业关注度数据。</EmptyState>}</Card>
        <Card title="录取规则与政策类问题统计">
          {special?.policyStats.length ? <div className="policy-bars">{special.policyStats.map(([name, count]) => <div key={name}><span>{count}</span><b>{name}</b></div>)}</div> : <EmptyState compact>暂无录取规则统计。</EmptyState>}
        </Card>
      </div>
      <Card title="专项与政策咨询详细数据">
        {special?.specialPlans.length ? <DataTable headers={['名称', '咨询量', '占比', '数据来源', '热度趋势']} rows={special.specialPlans.map((row) => [...row, '▁▃▅▆▇'])} /> : <EmptyState compact>暂无专项与政策咨询明细。</EmptyState>}
      </Card>
      <Card title="2021-2025 录取统计覆盖" action={<span className="soft-pill">来自录取统计表，不等同招生计划</span>}>
        {admissions?.stats.length ? <StatGrid stats={admissions.stats} /> : <EmptyState compact>暂无录取统计覆盖指标。</EmptyState>}
        <div className="grid-two">
          {admissions?.yearCounts.length ? <Chart option={barOption(admissions.yearCounts.map((item) => item.name), admissions.yearCounts.map((item) => item.value), '#2161ff')} height={260} /> : <EmptyState compact>暂无年份覆盖数据。</EmptyState>}
          {admissions?.provinceCoverage.length ? <Chart option={horizontalBarOption(admissions.provinceCoverage.slice(0, 10))} height={260} /> : <EmptyState compact>暂无省份覆盖数据。</EmptyState>}
        </div>
        {admissions?.topMajors.length ? <DataTable headers={['覆盖较多的专业', '覆盖省份数']} rows={admissions.topMajors.slice(0, 8).map(([name, count]) => [name, count])} /> : <EmptyState compact>暂无专业覆盖数据。</EmptyState>}
      </Card>
    </>
  );
}

function EvaluationOverviewPage() {
  return (
    <>
      <PageTitle title="测评数据总览" subtitle="志愿填报工具使用分析 · 考生刚需程度洞察" />
      <div className="segment"><button>今日</button><button className="active">近7天</button><button>近30天</button><button>全年</button></div>
      <div className="grid-two">
        <Card title="按地域维度 · 各省份测评统计">
          <EmptyState compact>测评统计接口尚未接入，暂不展示临时数据。</EmptyState>
        </Card>
        <Card title="每日测评使用量趋势">
          <EmptyState compact>暂无真实测评趋势数据。</EmptyState>
        </Card>
      </div>
      <div className="grid-three">
        <Card title="考生类型 · 科类分布"><EmptyState compact>暂无真实科类分布。</EmptyState></Card>
        <Card title="考生类型 · 报考类型分布"><EmptyState compact>暂无真实报考类型分布。</EmptyState></Card>
        <Card title="分数段 · 位次区间分布"><EmptyState compact>暂无真实分数段分布。</EmptyState></Card>
      </div>
      <Card title="深度业务洞察">
        <EmptyState compact>待测评数据接口接入后生成真实业务洞察。</EmptyState>
      </Card>
    </>
  );
}

function EvaluationPage() {
  const [query, setQuery] = useState('');
  return (
    <>
      <PageTitle title="测评明细" subtitle="每条记录存档、可溯源" />
      <Toolbar>
        <SearchBox value={query} onChange={setQuery} placeholder="搜索编号、省份、联系方式..." />
        <PrimaryButton ghost><Filter size={16} />筛选</PrimaryButton>
        <PrimaryButton ghost><Download size={16} />导出Excel</PrimaryButton>
      </Toolbar>
      <Card>
        <EmptyState>测评明细接口尚未接入，暂不展示临时记录。</EmptyState>
      </Card>
    </>
  );
}

function ConversationsPage() {
  const [query, setQuery] = useState('');
  const [apiRows, setApiRows] = useState<AdminConversationListItem[] | null>(null);
  const [selected, setSelected] = useState<AdminConversationListItem | null>(null);
  const [detail, setDetail] = useState<AdminConversationDetail | null>(null);
  const [loadError, setLoadError] = useState('');
  const [feedbackType, setFeedbackType] = useState<AdminFeedbackType>('manual-fix');
  const [feedbackComment, setFeedbackComment] = useState('');
  const [feedbackSaved, setFeedbackSaved] = useState('');
  const [feedbackSaving, setFeedbackSaving] = useState(false);
  const [loading, setLoading] = useState(true);
  const rows = apiRows ?? [];

  const loadConversations = useCallback(() => {
    setLoading(true);
    fetchAdminConversations(query)
      .then((data) => {
        setApiRows(data.items);
        setLoadError('');
      })
      .catch((error: Error) => {
        setApiRows([]);
        setLoadError(error.message);
      })
      .finally(() => {
        setLoading(false);
      });
  }, [query]);

  useEffect(() => {
    loadConversations();
  }, [loadConversations]);

  function openConversation(row: AdminConversationListItem) {
    setSelected(row);
    setDetail(null);
    setFeedbackComment('');
    setFeedbackSaved('');
    fetchAdminConversationDetail(row.id)
      .then(setDetail)
      .catch(() => setDetail(null));
  }

  async function submitFeedback() {
    if (!selected || !feedbackComment.trim()) {
      setFeedbackSaved('请先填写反馈备注。');
      return;
    }
    setFeedbackSaving(true);
    setFeedbackSaved('');
    try {
      await createAdminFeedback({
        conversationId: selected.id,
        feedbackType,
        comment: feedbackComment.trim(),
        handledBy: 'admin',
        status: feedbackType === 'helpful' ? 'resolved' : 'open',
      });
      setFeedbackSaved('人工反馈已写入，可在后续审计中跟踪处理。');
      setFeedbackComment('');
      fetchAdminConversations(query).then((data) => setApiRows(data.items)).catch(() => {});
    } catch (error) {
      setFeedbackSaved(error instanceof Error ? error.message : '人工反馈提交失败');
    } finally {
      setFeedbackSaving(false);
    }
  }

  return (
    <>
      <Toolbar>
        <SelectLike>近7天</SelectLike><SelectLike>全部省份</SelectLike><SelectLike>全部状态</SelectLike>
        <SearchBox value={query} onChange={setQuery} placeholder="搜索对话内容..." />
        <RefreshAction loading={loading} onClick={loadConversations} />
        <PrimaryButton ghost><Download size={16} />导出</PrimaryButton>
      </Toolbar>
      {loadError ? <LoadError message={loadError} onRetry={loadConversations} /> : null}
      <Card title={`对话记录列表 ${rows.length} 条`}>
        {loading && !rows.length ? <EmptyState compact>正在读取真实对话记录...</EmptyState> : null}
        {!loading && rows.length === 0 ? <EmptyState compact>暂无匹配的真实对话记录。</EmptyState> : null}
        {rows.length ? (
          <DataTable
            headers={['会话ID', '用户省份', '对话时间', '问题数', '状态', '人工介入', '最近问题', '操作']}
            rows={rows.map((row) => [
              row.id,
              row.province,
              row.updatedAt,
              row.messageCount,
              <StatusBadge value={row.status} />,
              row.manualIntervention ? '是' : '否',
              row.lastMessage,
              <button className="table-action" type="button" onClick={() => openConversation(row)}>查看</button>,
            ])}
          />
        ) : null}
      </Card>
      {selected ? (
        <Modal title={`会话审计 ${selected.id}`} onClose={() => { setSelected(null); setDetail(null); }}>
          <p className="dialog-text">用户来自{selected.province}，共记录 {detail?.messageCount ?? selected.messageCount} 条消息。人工介入：{selected.manualIntervention ? '是' : '否'}。</p>
          {detail ? (
            <div className="dialog-thread">
              {detail.messages.map((message, index) => (
                <div className={`dialog-bubble ${message.role}`} key={`${message.role}-${index}`}>
                  <b>{message.role === 'assistant' ? '助手' : '用户'}</b>
                  <p>{message.content}</p>
                  {message.createdAt ? <time>{message.createdAt}</time> : null}
                </div>
              ))}
            </div>
          ) : (
            <EmptyState>正在读取真实对话详情。</EmptyState>
          )}
          <div className="feedback-panel">
            <h3>人工反馈 / 纠错</h3>
            <div className="feedback-grid">
              <label className="field"><span>反馈类型</span>
                <select value={feedbackType} onChange={(event) => setFeedbackType(event.target.value as AdminFeedbackType)}>
                  <option value="manual-fix">人工纠错</option>
                  <option value="incorrect">回答有误</option>
                  <option value="helpful">回答有帮助</option>
                </select>
              </label>
              <label className="field"><span>处理备注</span><textarea value={feedbackComment} onChange={(event) => setFeedbackComment(event.target.value)} placeholder="记录问题、建议处理方式或确认结论。" /></label>
            </div>
            <PrimaryButton onClick={() => void submitFeedback()}><Save size={16} />{feedbackSaving ? '提交中...' : '提交反馈'}</PrimaryButton>
            {feedbackSaved ? <span className="save-tip">{feedbackSaved}</span> : null}
          </div>
        </Modal>
      ) : null}
    </>
  );
}

function KnowledgePage() {
  const [query, setQuery] = useState('');
  const [apiItems, setApiItems] = useState<KnowledgeItem[] | null>(null);
  const [chunks, setChunks] = useState<AdminKnowledgeChunkItem[]>([]);
  const [coverage, setCoverage] = useState<AdminKnowledgeCoverageSnapshot | null>(null);
  const [faqTotal, setFaqTotal] = useState<number | null>(null);
  const [chunkTotal, setChunkTotal] = useState<number | null>(null);
  const [loadError, setLoadError] = useState('');
  const [loading, setLoading] = useState(true);
  const [modal, setModal] = useState<'new' | 'import' | 'edit' | null>(null);
  const [editingFaq, setEditingFaq] = useState<KnowledgeItem | null>(null);
  const [savingFaq, setSavingFaq] = useState(false);
  const rows = apiItems ?? [];

  const loadKnowledge = useCallback(() => {
    setLoading(true);
    Promise.all([
      fetchAdminFaqs(query),
      fetchAdminKnowledgeChunks(query),
      fetchAdminKnowledgeCoverage(),
    ])
      .then(([faqList, chunkList, coverageData]) => {
        setApiItems(faqList.items);
        setChunks(chunkList.items);
        setCoverage(coverageData);
        setFaqTotal(faqList.total);
        setChunkTotal(chunkList.total);
        setLoadError('');
      })
      .catch((error: Error) => {
        setApiItems([]);
        setChunks([]);
        setCoverage(null);
        setFaqTotal(null);
        setChunkTotal(null);
        setLoadError(error.message);
      })
      .finally(() => {
        setLoading(false);
      });
  }, [query]);

  useEffect(() => {
    loadKnowledge();
  }, [loadKnowledge]);

  function openFaqEditor(item: KnowledgeItem) {
    setEditingFaq(item);
    setModal('edit');
  }

  async function saveFaqDraft(input: {
    question: string;
    answer: string;
    category: string;
    tags: string[];
    status: 'draft' | 'published';
    sourceLabel: string;
  }) {
    setSavingFaq(true);
    try {
      const saved = modal === 'edit' && editingFaq
        ? await updateAdminFaq(String(editingFaq.id), input)
        : await createAdminFaq(input);
      setApiItems((current) => {
        const list = current ?? [];
        return modal === 'edit'
          ? list.map((item) => String(item.id) === String(saved.id) ? saved : item)
          : [saved, ...list];
      });
      setModal(null);
      setEditingFaq(null);
      setLoadError('');
    } catch (error) {
      setLoadError(error instanceof Error ? error.message : 'FAQ 保存失败');
    } finally {
      setSavingFaq(false);
    }
  }

  return (
    <>
      <Toolbar>
        <SelectLike>全部</SelectLike>
        <SearchBox value={query} onChange={setQuery} placeholder="搜索问答内容..." />
        <RefreshAction loading={loading} onClick={loadKnowledge} />
        <PrimaryButton ghost onClick={() => setModal('import')}><Upload size={16} />批量导入</PrimaryButton>
        <PrimaryButton ghost><Download size={16} />导出</PrimaryButton>
        <PrimaryButton onClick={() => { setEditingFaq(null); setModal('new'); }}><Plus size={16} />新增 FAQ</PrimaryButton>
      </Toolbar>
      {loadError ? <LoadError message={loadError} onRetry={loadKnowledge} /> : null}
      {coverage ? (
        <>
        <Card title="知识库真实覆盖概览" action={<span className="soft-pill">更新时间：{coverage.updatedAt}</span>}>
          <StatGrid stats={coverage.stats} />
        </Card>
        <div className="grid-two knowledge-chart-row">
          <Card title="文档类型分布">
            {coverage.documentKinds.length ? <Chart option={centeredDonutOption(coverage.documentKinds)} height={300} /> : <EmptyState compact>暂无文档类型数据。</EmptyState>}
          </Card>
          <Card title="政策文档年份分布">
            {coverage.policyYears.length ? <Chart option={barOption(coverage.policyYears.map((item) => item.name), coverage.policyYears.map((item) => item.value), '#34c8c2')} height={300} /> : <EmptyState compact>暂无政策年份数据。</EmptyState>}
          </Card>
        </div>
        <Card title="培养方案学院覆盖" action={<span className="soft-pill">按文档片段数量统计</span>}>
          {coverage.collegeChunks.length ? <Chart option={wideHorizontalBarOption(coverage.collegeChunks.slice(0, 12))} height={360} /> : <EmptyState compact>暂无学院覆盖数据。</EmptyState>}
        </Card>
        <Card title="FAQ 分类覆盖" action={<span className="soft-pill">按固定问答分类统计</span>}>
          <div className="knowledge-centered-chart">
            {coverage.faqCategories.length ? <Chart option={centeredDonutOption(coverage.faqCategories)} height={420} /> : <EmptyState compact>暂无 FAQ 分类数据。</EmptyState>}
          </div>
        </Card>
        </>
      ) : null}
      <Card title={`标准问答库 ${faqTotal ?? rows.length} 条`}>
        {loading && !rows.length ? <EmptyState compact>正在读取真实 FAQ...</EmptyState> : null}
        {!loading && rows.length === 0 ? <EmptyState compact>暂无匹配的真实 FAQ。</EmptyState> : null}
        {rows.length ? <DataTable headers={['ID', '标准问题', '相似问法', '标准答案', '来源', '更新时间', '状态', '命中', '操作']} rows={rows.map((item) => [item.id, item.question, item.similar, item.answer, item.source, item.updatedAt, <StatusBadge value={item.status} />, item.hits, <button className="table-action" type="button" onClick={() => openFaqEditor(item)}>编辑</button>])} /> : null}
      </Card>
      <Card title={`PDF 文档片段审计 ${chunkTotal ?? chunks.length} 条`} action={<span className="soft-pill">招生简章 / 培养方案</span>}>
        {chunks.length ? (
          <div className="chunk-list">
            {chunks.map((chunk) => (
              <article key={chunk.id}>
                <div className="chunk-meta">
                  <b>{chunk.title || chunk.documentKind || '未命名片段'}</b>
                  <span>{chunk.college || '全校'}</span>
                  {chunk.majorName ? <span>{chunk.majorName}</span> : null}
                  <time>{chunk.updatedAt}</time>
                </div>
                <p>{chunk.excerpt}</p>
              </article>
            ))}
          </div>
        ) : (
          <EmptyState compact>没有匹配到真实文档片段。</EmptyState>
        )}
      </Card>
      <Card title="未知问题归集池" action={<span className="soft-pill amber">待接入真实事件日志</span>}>
        <EmptyState compact>未知问题归集需要接入低置信度回答、无证据回答、人工反馈等真实事件日志；当前不展示临时数据。</EmptyState>
      </Card>
      {modal ? <KnowledgeModal mode={modal} initialItem={editingFaq} saving={savingFaq} onClose={() => { setModal(null); setEditingFaq(null); }} onSave={(item) => void saveFaqDraft(item)} /> : null}
    </>
  );
}

function KnowledgeModal({
  mode,
  initialItem,
  saving,
  onClose,
  onSave,
}: {
  mode: 'new' | 'import' | 'edit';
  initialItem: KnowledgeItem | null;
  saving: boolean;
  onClose: () => void;
  onSave: (item: {
    question: string;
    answer: string;
    category: string;
    tags: string[];
    status: 'draft' | 'published';
    sourceLabel: string;
  }) => void;
}) {
  const [question, setQuestion] = useState(initialItem?.question ?? '');
  const [answer, setAnswer] = useState(initialItem?.answer ?? '');
  const [category, setCategory] = useState(initialItem?.source || '招生咨询');
  const [tagsText, setTagsText] = useState(initialItem?.similar ?? '');
  const [status, setStatus] = useState<'draft' | 'published'>(initialItem?.status === '启用' ? 'published' : 'draft');
  const [sourceLabel, setSourceLabel] = useState(initialItem?.source ?? '管理后台录入');
  const tags = tagsText.split(/[|,，]/).map((item) => item.trim()).filter(Boolean);

  return (
    <Modal title={mode === 'edit' ? '编辑标准问答' : mode === 'new' ? '新增标准问答' : '批量导入问答'} onClose={onClose}>
      {mode === 'import' ? <div className="upload-drop"><Upload size={30} />拖拽 Excel 文件到此处，或点击选择文件</div> : null}
      <label className="field"><span>标准问题</span><input value={question} onChange={(event) => setQuestion(event.target.value)} placeholder="请输入标准问题" /></label>
      <label className="field"><span>标准答案</span><textarea value={answer} onChange={(event) => setAnswer(event.target.value)} placeholder="请输入标准答案" /></label>
      <div className="grid-two compact-grid">
        <label className="field"><span>分类</span><input value={category} onChange={(event) => setCategory(event.target.value)} /></label>
        <label className="field"><span>状态</span>
          <select value={status} onChange={(event) => setStatus(event.target.value as 'draft' | 'published')}>
            <option value="draft">草稿，不进入正式 FAQ</option>
            <option value="published">发布，进入 FAQ 检索候选</option>
          </select>
        </label>
      </div>
      <label className="field"><span>相似问法 / 标签</span><input value={tagsText} onChange={(event) => setTagsText(event.target.value)} placeholder="用逗号或 | 分隔" /></label>
      <label className="field"><span>来源标签</span><input value={sourceLabel} onChange={(event) => setSourceLabel(event.target.value)} /></label>
      <p className="soft-note">保存 FAQ 只更新结构化问答表；向量 chunk / embedding 后续通过单独的审核重嵌入流程处理。</p>
      <PrimaryButton onClick={() => onSave({ question, answer, category, tags, status, sourceLabel })}><Save size={16} />{saving ? '保存中...' : '保存 FAQ'}</PrimaryButton>
    </Modal>
  );
}

function TicketsPage() {
  const [active, setActive] = useState('全部');
  const [query, setQuery] = useState('');
  const [list, setList] = useState<AdminTicketItem[] | null>(null);
  const [loadError, setLoadError] = useState('');
  const [loading, setLoading] = useState(true);
  const [ticket, setTicket] = useState<AdminTicketItem | null>(null);
  const [resolution, setResolution] = useState('');
  const tabs = ['全部', '待处理', '处理中', '已办结'];

  const loadTickets = useCallback(() => {
    setLoading(true);
    fetchAdminTickets(query, active)
      .then((data) => {
        setList(data.items);
        setLoadError('');
      })
      .catch((error: Error) => {
        setList([]);
        setLoadError(error.message);
      })
      .finally(() => {
        setLoading(false);
      });
  }, [active, query]);

  useEffect(() => {
    loadTickets();
  }, [loadTickets]);

  const rows = list ?? [];
  const filtered = rows.filter((item) => {
    const phone = item.phone ?? '';
    const email = item.email ?? '';
    return (active === '全部' || item.status === active) && `${item.name}${phone}${email}${item.content}${item.province}`.includes(query);
  });

  function openTicket(item: AdminTicketItem) {
    setTicket(item);
    setResolution(item.resolution ? item.resolution : '');
  }

  async function advance(id: string) {
    const currentStatus = ticket?.status ?? '待处理';
    const nextStatus = currentStatus === '待处理' ? '处理中' : '已办结';
    if (!list) {
      setTicket(null);
      return;
    }
    try {
      const updated = await updateAdminTicket(id, {
        status: nextStatus,
        resolution: resolution.trim() || undefined,
        handledBy: 'admin',
      });
      setList((current) => current?.map((item) => item.id === id ? updated : item) ?? current);
      setTicket(null);
    } catch (error) {
      setLoadError(error instanceof Error ? error.message : '工单更新失败');
    }
  }

  return (
    <>
      <Toolbar>
        <div className="tabs">{tabs.map((tab) => <button className={active === tab ? 'active' : ''} type="button" onClick={() => setActive(tab)} key={tab}>{tab}</button>)}</div>
        <SearchBox value={query} onChange={setQuery} placeholder="搜索姓名/电话/邮箱/内容..." />
        <RefreshAction loading={loading} onClick={loadTickets} />
      </Toolbar>
      {loadError ? <LoadError message={loadError} onRetry={loadTickets} /> : null}
      <Card title={`留言工单列表 ${filtered.length} 条`}>
        {loading && !filtered.length ? <EmptyState compact>正在读取真实留言工单...</EmptyState> : null}
        {!loading && !filtered.length ? <EmptyState>当前没有匹配的真实工单。</EmptyState> : null}
        {filtered.length ? (
          <DataTable headers={['工单编号', '姓名', '省份', '联系方式', '咨询内容', '提交时间', '状态', '优先级', '操作']} rows={filtered.map((item) => {
            const phone = item.phone;
            const email = item.email;
            return [
              item.id,
              item.name,
              item.province,
              <div className="stacked-cell">
                <span>{phone || '未留手机'}</span>
                {email ? <small>{email}</small> : null}
              </div>,
              item.content,
              item.createdAt,
              <StatusBadge value={item.status} />,
              <StatusBadge value={item.priority} />,
              <button className="table-action" type="button" onClick={() => openTicket(item)}>办理</button>
            ];
          })} />
        ) : null}
      </Card>
      {ticket ? <Modal title={`工单 ${ticket.id}`} onClose={() => setTicket(null)}>
        <div className="detail-grid">
          <div><span>姓名</span><b>{ticket.name}</b></div>
          <div><span>省份</span><b>{ticket.province}</b></div>
          <div><span>手机</span><b>{ticket.phone ? ticket.phone : '未填写'}</b></div>
          <div><span>邮箱</span><b>{ticket.email ? ticket.email : '未填写'}</b></div>
        </div>
        <p className="dialog-text">{ticket.content}</p>
        <label className="field"><span>处理备注</span><textarea value={resolution} onChange={(event) => setResolution(event.target.value)} placeholder="填写处理结论、回访情况或下一步安排。" /></label>
        <PrimaryButton onClick={() => void advance(ticket.id)}><CheckCircle2 size={16} />推进状态</PrimaryButton>
      </Modal> : null}
    </>
  );
}

function SettingsPage() {
  const [tab, setTab] = useState('数字人配置');
  const [welcome, setWelcome] = useState('');
  const [fallback, setFallback] = useState('');
  const [saved, setSaved] = useState(false);
  const [settings, setSettings] = useState<AdminSettings | null>(null);
  const [auditLogs, setAuditLogs] = useState<AdminAuditLogItem[]>([]);
  const [loadError, setLoadError] = useState('');
  const [loading, setLoading] = useState(true);

  const loadSettings = useCallback(() => {
    setLoading(true);
    Promise.all([fetchAdminSettings(), fetchAdminAuditLogs()])
      .then(([settingsData, auditData]) => {
        setSettings(settingsData);
        setWelcome(settingsData.welcomeMessage);
        setFallback(settingsData.fallbackMessage);
        setAuditLogs(auditData.items);
        setLoadError('');
      })
      .catch((error: Error) => {
        setSettings(null);
        setAuditLogs([]);
        setLoadError(error.message);
      })
      .finally(() => {
        setLoading(false);
      });
  }, []);

  useEffect(() => {
    loadSettings();
  }, [loadSettings]);

  async function saveSettings() {
    setSaved(false);
    try {
      const updated = await updateAdminSettings({
        welcomeMessage: welcome,
        fallbackMessage: fallback,
        updatedBy: 'admin',
      });
      setSettings(updated);
      setWelcome(updated.welcomeMessage);
      setFallback(updated.fallbackMessage);
      setSaved(true);
      fetchAdminAuditLogs().then((data) => setAuditLogs(data.items)).catch(() => {});
    } catch (error) {
      setLoadError(error instanceof Error ? error.message : '配置保存失败');
    }
  }

  return (
    <>
      <div className="tabs settings-tabs">
        {['数字人配置', '热门问题', '账号管理', '操作日志'].map((item) => <button className={tab === item ? 'active' : ''} type="button" onClick={() => setTab(item)} key={item}>{item}</button>)}
        <RefreshAction loading={loading} onClick={loadSettings} />
      </div>
      {loadError ? <LoadError message={loadError} onRetry={loadSettings} /> : null}
      {tab === '数字人配置' ? (
        <div className="grid-two">
          <Card title="数字人欢迎语配置">
            {loading && !settings ? <EmptyState compact>正在读取真实配置...</EmptyState> : null}
            <label className="field"><span>开场欢迎语</span><textarea value={welcome} onChange={(event) => setWelcome(event.target.value)} /></label>
            <label className="field"><span>兜底话术配置</span><textarea value={fallback} onChange={(event) => setFallback(event.target.value)} /></label>
            <PrimaryButton onClick={() => void saveSettings()}><Save size={16} />保存配置</PrimaryButton>
            {saved ? <span className="save-tip">配置已保存，预览已同步更新</span> : null}
            {settings?.updatedAt ? <p className="soft-note">最近保存：{settings.updatedAt}</p> : null}
          </Card>
          <Card title="配置预览">
            <div className="preview-card"><img src="/assets/muyang.gif" alt="沐阳" /><b>沐阳</b><p>{welcome}</p><small>兜底：{fallback}</small></div>
          </Card>
        </div>
      ) : tab === '操作日志' ? (
        <Card title="操作日志">
          {auditLogs.length ? (
            <DataTable headers={['时间', '操作', '对象', '操作者', '详情']} rows={auditLogs.map((item) => [item.createdAt, item.action, item.targetType, item.actor, JSON.stringify(item.detail)])} />
          ) : (
            <div className="empty-state">暂无后台操作日志。</div>
          )}
        </Card>
      ) : (
        <Card title={tab}><div className="empty-state">该模块已按源站样式预留，可继续扩展真实配置项。</div></Card>
      )}
    </>
  );
}

function BigScreenPage() {
  const [category, setCategory] = useState('总榜');
  const [screen, setScreen] = useState<AdminBigScreenSnapshot | null>(null);
  const [loadError, setLoadError] = useState('');
  const [loading, setLoading] = useState(true);
  const now = new Date().toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }).replace(/\//g, '/');
  const bigStatIcons = [Users, MessageSquare, Globe2, LayoutDashboard, MapPinned, Target, FileText];
  const screenMapData = screen?.mapData ?? [];
  const provinceTotal = screenMapData.reduce((sum, item) => sum + item.value, 0) || 1;
  const provincePercents = screenMapData.map((item) => `${(item.value * 100 / provinceTotal).toFixed(1)}%`);
  const provinceEvaluate = screenMapData.map((item) => Math.max(Math.round(item.value * 0.28), 0));
  const behaviorData = screen?.behaviorCards.map((item) => [item.label, item.value, item.delta, item.points] as const) ?? [];
  const screenStats = screen?.bigStats ?? [];
  const screenRealtimeMessages = screen?.realtimeMessages ?? [];
  const screenTopQuestions = screen?.topQuestions.map((item) => [item.question, `${item.count}次`, item.share] as [string, string, string]) ?? [];
  const mapMax = Math.max(...screenMapData.map((item) => item.value), 1);

  const loadBigScreen = useCallback(() => {
    setLoading(true);
    fetchAdminBigScreen()
      .then((data) => {
        setScreen(data);
        setLoadError('');
      })
      .catch((error: Error) => {
        setScreen(null);
        setLoadError(error.message);
      })
      .finally(() => {
        setLoading(false);
      });
  }, []);

  useEffect(() => {
    loadBigScreen();
  }, [loadBigScreen]);

  const mapOption = {
    animation: false,
    backgroundColor: 'transparent',
    tooltip: { trigger: 'item' },
    visualMap: {
      min: 0,
      max: mapMax,
      right: 18,
      bottom: 120,
      text: ['高', '低'],
      textStyle: { color: '#a8c7ff' },
      inRange: { color: ['#63a6ff', '#8cf06e', '#ffd04f', '#ff5555'] },
    },
    series: [{
      name: '咨询量',
      type: 'map',
      map: 'china',
      roam: false,
      zoom: 1,
      layoutCenter: ['50%', '54%'],
      layoutSize: '96%',
      label: { show: false },
      itemStyle: { areaColor: '#2a78d7', borderColor: '#163f8a', borderWidth: 0.8 },
      emphasis: { itemStyle: { areaColor: '#ff9d24' } },
      data: screenMapData,
    }],
  };

  return (
    <main className="big-screen">
      <header className="big-header">
        <div><b>哈师大招生智能体管理后台</b><span>{now}</span></div>
        <h1>哈尔滨师范大学 · 招生智能体<small>全国生源招生咨询态势总览</small></h1>
        <p>{screen ? `真实咨询数据更新时间：${screen.updatedAt}` : loading ? '正在读取真实咨询数据' : '暂无真实咨询数据'}</p>
      </header>
      {loadError ? <LoadError message={loadError} onRetry={loadBigScreen} /> : null}
      <div className="big-stat-row">
        {screenStats.length ? screenStats.map((stat, index) => {
          const Icon = bigStatIcons[index] ?? Target;
          return (
            <div key={stat.label}>
              <i><Icon size={20} /></i>
              <span>{stat.label}</span>
              <strong>{stat.value}</strong>
            </div>
          );
        }) : <EmptyState compact>暂无真实大屏指标。</EmptyState>}
      </div>
      <section className="big-grid">
        <aside className="big-left">
          <BigPanel title="用户行为数据统计" action={<SelectLike>近30天</SelectLike>}>
            <div className="behavior-grid">
              {behaviorData.length ? behaviorData.map(([label, value, delta, points], index) => (
                <div key={label}>
                  <span>{label}</span>
                  <strong>{value}<em>↑{delta.replace('+', '')}</em></strong>
                  <SparkLine points={points} tone={index % 3 === 2 ? 'orange' : index % 3 === 1 ? 'blue' : 'green'} />
                </div>
              )) : <EmptyState compact>暂无真实用户行为数据。</EmptyState>}
            </div>
          </BigPanel>
          <BigPanel title="各省咨询数据详情" action={<button type="button" className="export-action"><Download size={15} />导出</button>}>
            <div className="province-detail">
              <div className="donut"><strong>{screenMapData.length}</strong><span>省份</span></div>
              <div className="province-table">
                <div className="province-table-head"><span>省份</span><span>咨询量</span><span>测评量</span></div>
                {screenMapData.length ? <ul>{screenMapData.map((item, index) => (
                  <li key={item.name}>
                    <span><i style={{ background: `hsl(212, 95%, ${68 - (index % 5) * 6}%)` }} />{item.name}{['上海', '北京'].includes(item.name) ? '市' : '省'}</span>
                    <b>{item.value.toLocaleString()} <small>({provincePercents[index] ?? '0.0%'})</small></b>
                    <em>{(provinceEvaluate[index] ?? 0).toLocaleString()} <small>({provincePercents[index] ?? '0.0%'})</small></em>
                  </li>
                ))}</ul> : <EmptyState compact>暂无真实省份分布数据。</EmptyState>}
                <div className="province-scrollbar"><span /><b /></div>
              </div>
            </div>
          </BigPanel>
        </aside>
        <section className="map-area">
          <BigPanel title="全国各省咨询热力分布" action={<span>IP解析精准度≥99.9%</span>}>
            {screenMapData.length ? <Chart option={mapOption} height={390} /> : <EmptyState compact>暂无真实热力图数据。</EmptyState>}
          </BigPanel>
          <BigPanel title="实时咨询动态" action={<span className="live-dot">实时</span>}>
            {screenRealtimeMessages.length ? <div className="live-list">
              <div className="live-track">
                {[...screenRealtimeMessages, ...screenRealtimeMessages].map(({ province, question, answer, time }, index) => (
                  <div key={`${province}-${time}-${index}`}>
                    <b>{province}</b>
                    <span>{question}</span>
                    <em>答：{answer}</em>
                    <time>{time}</time>
                  </div>
                ))}
              </div>
            </div> : <EmptyState compact>暂无真实实时咨询动态。</EmptyState>}
          </BigPanel>
        </section>
        <aside className="big-right">
          <BigPanel title="全国热点问题TOP榜" action={<RefreshAction loading={loading} onClick={loadBigScreen} />}>
            <div className="big-tabs">{['总榜 (100%)', '院校介绍 (25%)', '分数与位次 (32%)', '招生计划 (18%)', '录取政策 (12%)', '专项与公费师范 (8%)', '就读与就业 (5%)'].map((item) => <button className={category === item.split(' ')[0] ? 'active' : ''} type="button" onClick={() => setCategory(item.split(' ')[0])} key={item}>{item}</button>)}</div>
            <div className="big-insight">
              <b>智能洞察</b>
              {screen?.insight ? <p>{screen.insight}</p> : <EmptyState compact>暂无真实智能洞察。</EmptyState>}
            </div>
            {screenTopQuestions.length ? <ol className="big-rank">{screenTopQuestions.map(([question, count, delta], index) => <li key={question}><b>{index + 1}</b><span>{question}</span><em>{count}</em><strong className={delta.startsWith('-') ? 'negative' : ''}>{delta}</strong></li>)}</ol> : <EmptyState compact>暂无真实热点榜单。</EmptyState>}
          </BigPanel>
        </aside>
      </section>
    </main>
  );
}

function SparkLine({ points, tone }: { points: readonly number[]; tone: 'green' | 'blue' | 'orange' }) {
  const max = Math.max(...points);
  const min = Math.min(...points);
  const range = Math.max(max - min, 1);
  const d = points
    .map((point, index) => {
      const x = 8 + index * (88 / Math.max(points.length - 1, 1));
      const y = 30 - ((point - min) / range) * 18;
      return `${index === 0 ? 'M' : 'L'} ${x.toFixed(1)} ${y.toFixed(1)}`;
    })
    .join(' ');

  return (
    <svg className={`spark-line ${tone}`} viewBox="0 0 108 36" aria-hidden="true">
      <path d={d} />
    </svg>
  );
}

function MiniMetric({ icon: Icon, label, value }: { icon: LucideIcon; label: string; value: string }) {
  return <div className="mini-metric"><Icon size={22} /><span>{label}</span><strong>{value}</strong></div>;
}

function DataTable({ headers, rows }: { headers: ReactNode[]; rows: ReactNode[][] }) {
  return (
    <div className="table-wrap">
      <table>
        <thead><tr>{headers.map((header, index) => <th key={`${String(header)}-${index}`}>{header}</th>)}</tr></thead>
        <tbody>{rows.map((row, rowIndex) => <tr key={rowIndex}>{row.map((cell, index) => <td key={index}>{cell}</td>)}</tr>)}</tbody>
      </table>
    </div>
  );
}

function BigPanel({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  return <section className="big-panel"><div className="big-panel-head"><h2>{title}</h2>{action}</div>{children}</section>;
}

function Chart({ option, height }: { option: object; height: number }) {
  return <ReactECharts option={option} style={{ height, width: '100%' }} notMerge lazyUpdate />;
}

function lineOption(labels: string[], values: number[], color: string) {
  return {
    animation: false,
    grid: { left: 45, right: 20, top: 35, bottom: 38 },
    tooltip: { trigger: 'axis' },
    xAxis: { type: 'category', data: labels.length === values.length ? labels : values.map((_, index) => labels[index % labels.length]), axisTick: { show: false } },
    yAxis: { type: 'value', splitLine: { lineStyle: { color: '#eef1f6' } } },
    series: [{ type: 'line', smooth: true, data: values, symbolSize: 8, itemStyle: { color }, lineStyle: { width: 3, color }, areaStyle: { color: `${color}1a` } }],
  };
}

function barOption(labels: string[], values: number[], color: string) {
  return {
    animation: false,
    grid: { left: 45, right: 20, top: 35, bottom: 38 },
    tooltip: { trigger: 'axis' },
    xAxis: { type: 'category', data: labels, axisTick: { show: false } },
    yAxis: { type: 'value', splitLine: { lineStyle: { color: '#eef1f6' } } },
    series: [{ type: 'bar', data: values, itemStyle: { color, borderRadius: [5, 5, 0, 0] }, barWidth: '48%' }],
  };
}

function horizontalBarOption(values: Array<[string, number]>) {
  return {
    animation: false,
    grid: { left: 90, right: 20, top: 20, bottom: 25 },
    tooltip: { trigger: 'axis' },
    xAxis: { type: 'value', splitLine: { lineStyle: { color: '#eef1f6' } } },
    yAxis: { type: 'category', data: values.map((item) => item[0]), axisTick: { show: false } },
    series: [{ type: 'bar', data: values.map((item) => item[1]), itemStyle: { color: '#35c7c4', borderRadius: [0, 5, 5, 0] }, barWidth: 18 }],
  };
}

function wideHorizontalBarOption(values: Array<[string, number]>) {
  return {
    animation: false,
    grid: { left: 170, right: 32, top: 18, bottom: 32 },
    tooltip: { trigger: 'axis' },
    xAxis: { type: 'value', splitLine: { lineStyle: { color: '#eef1f6' } } },
    yAxis: {
      type: 'category',
      data: values.map((item) => item[0]),
      axisTick: { show: false },
      axisLabel: { color: '#475467', width: 150, overflow: 'truncate' },
    },
    series: [{ type: 'bar', data: values.map((item) => item[1]), itemStyle: { color: '#35c7c4', borderRadius: [0, 5, 5, 0] }, barWidth: 20 }],
  };
}

function centeredDonutOption(values: Array<{ name: string; value: number }>) {
  return {
    animation: false,
    tooltip: { trigger: 'item' },
    legend: {
      type: 'scroll',
      bottom: 0,
      left: 'center',
      orient: 'horizontal',
      itemWidth: 14,
      itemHeight: 10,
      textStyle: { color: '#475467', fontSize: 12 },
      pageIconColor: '#2161ff',
      pageTextStyle: { color: '#667085' },
    },
    color: ['#2161ff', '#35c7c4', '#ffb020', '#ff6b6b', '#8b5cf6', '#94a3b8', '#10b981', '#f97316', '#0ea5e9', '#ec4899', '#64748b', '#a855f7'],
    series: [{
      type: 'pie',
      radius: ['42%', '66%'],
      center: ['50%', '43%'],
      data: values,
      avoidLabelOverlap: true,
      label: { show: false },
      labelLine: { show: false },
      emphasis: {
        label: {
          show: true,
          formatter: '{b}\n{d}%',
          color: '#101828',
          fontSize: 13,
          fontWeight: 700,
        },
      },
    }],
  };
}

function pieOption(values: Array<{ name: string; value: number }>) {
  return {
    animation: false,
    tooltip: { trigger: 'item' },
    legend: { right: 12, top: 'middle', orient: 'vertical' },
    color: ['#2161ff', '#35c7c4', '#ffb020', '#ff6b6b', '#8b5cf6', '#94a3b8'],
    series: [{ type: 'pie', radius: ['48%', '72%'], center: ['38%', '52%'], data: values, label: { formatter: '{b}\\n{d}%' } }],
  };
}

export default App;
