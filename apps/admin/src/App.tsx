import { useEffect, useState, type FormEvent, type ReactNode } from 'react';
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
  bigStats,
  bigTopQuestions,
  categoryStats,
  conversations,
  dashboardStats,
  evaluationRecords,
  hotQuestions,
  hourlyValues,
  insightMonths,
  knowledgeBase,
  majorAttention,
  mapData,
  overviewStats,
  provinceBars,
  provinceEvaluations,
  realtimeMessages,
  specialPlans,
  specialStats,
  tickets,
  top20,
  trendDays,
  trendValues,
  unknownQuestions,
  type EvaluationRecord,
  type KnowledgeItem,
  type Stat,
  type Ticket,
} from './data/mock';
import {
  fetchAdminConversationDetail,
  fetchAdminConversations,
  fetchAdminDashboard,
  fetchAdminFaqs,
  fetchAdminKnowledgeChunks,
} from './api/admin';
import type {
  AdminConversationDetail,
  AdminConversationListItem,
  AdminDashboardSnapshot,
  AdminKnowledgeChunkItem,
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

  useEffect(() => {
    let alive = true;
    fetchAdminDashboard()
      .then((data) => {
        if (alive) {
          setDashboard(data);
          setLoadError('');
        }
      })
      .catch((error: Error) => {
        if (alive) {
          setLoadError(error.message);
        }
      });
    return () => {
      alive = false;
    };
  }, []);

  const stats = dashboard?.stats ?? dashboardStats;
  const dashboardTrendDays = dashboard?.trendDays ?? trendDays;
  const dashboardTrendValues = dashboard?.trendValues ?? trendValues;
  const dashboardHourlyValues = dashboard?.hourlyValues ?? hourlyValues;
  const dashboardHotQuestions = dashboard?.hotQuestions ?? hotQuestions;

  return (
    <>
      <div className="page-meta">
        <SelectLike>近7天</SelectLike>
        <span><Clock3 size={16} /> 数据更新时间：{dashboard?.updatedAt ?? 'mock 数据'}</span>
        {loadError ? <span className="soft-pill amber">真实数据暂不可用，已显示本地样例</span> : null}
      </div>
      <StatGrid stats={stats} />
      <div className="grid-two">
        <Card title="咨询量趋势" action={<span className="soft-pill">↗ 上涨趋势</span>}>
          <Chart option={lineOption(dashboardTrendDays, dashboardTrendValues, '#2161ff')} height={290} />
        </Card>
        <Card title="24小时咨询时段分布">
          <Chart option={barOption(Array.from({ length: 24 }, (_, index) => `${index.toString().padStart(2, '0')}`), dashboardHourlyValues, '#34c8c2')} height={290} />
        </Card>
      </div>
      <div className="grid-two">
        <Card title="近期热点问题 TOP5" action={<a className="linkish" href="/insights">查看全部</a>}>
          <ol className="rank-list">
            {dashboardHotQuestions.slice(0, 5).map(([question, count], index) => <li key={question}><b>{index + 1}</b><span>{question}</span><em>{count}</em></li>)}
          </ol>
        </Card>
        <Card title="预警信息与快捷入口">
          <div className="warning-box"><AlertCircle size={18} />检测到5条敏感问题，请及时审核处理</div>
          <div className="warning-box"><Clock3 size={18} />23条工单待处理，其中8条超过24小时未响应</div>
          <div className="quick-grid">
            {['高频问题分析', '工单待办', '知识库更新', '待审核对话'].map((item) => <button type="button" key={item}>{item}</button>)}
          </div>
        </Card>
      </div>
    </>
  );
}

function InsightsPage() {
  const [month, setMonth] = useState(insightMonths[3]);
  const [query, setQuery] = useState('');
  const filtered = top20.filter((row) => row[0].includes(query) || row[1].includes(query));

  return (
    <>
      <PageTitle title="2026级新生招生 用户咨询洞察" />
      <Card>
        <div className="month-tabs">
          {insightMonths.map((item) => <button className={item.label === month.label ? 'active' : ''} type="button" key={item.label} onClick={() => setMonth(item)}>{item.label}</button>)}
          <span className="trend-alert">↗ 咨询趋势：高峰</span>
        </div>
        <div className="insight-hero">
          <MiniMetric icon={Users} label="咨询用户数" value={month.users} />
          <MiniMetric icon={MessageSquare} label="咨询问答数" value={month.questions} />
          <MiniMetric icon={Target} label="高意向留资量" value={month.leads} />
          <p><b>本月特征：</b>{month.summary}</p>
          <p><b>重点关注：</b>本月咨询量突破11万人次，创下年度单月最高纪录，整体咨询量较上月增长21%。省内咨询占比稳定在65%左右，但省外咨询增速明显加快。</p>
          <div className="tag-row"><span>二模成绩分析</span><span>各分数段报考建议</span><span>专业选择困惑</span><span>历年录取位次</span></div>
        </div>
      </Card>
      <Toolbar>
        <SelectLike>近30天</SelectLike>
        <SelectLike>全部分类</SelectLike>
        <SearchBox value={query} onChange={setQuery} placeholder="搜索问题关键词..." />
        <PrimaryButton ghost><Download size={16} />导出Excel</PrimaryButton>
      </Toolbar>
      <div className="grid-two">
        <Card title="咨询内容分类统计"><Chart option={pieOption(categoryStats)} height={280} /></Card>
        <Card title="生源地域热度分析"><Chart option={horizontalBarOption(provinceBars)} height={280} /></Card>
      </div>
      <Card title="高频问题 TOP20 榜单">
        <DataTable headers={['排名', '问题内容', '分类', '提问次数', '占比']} rows={filtered.map((row, index) => [index + 1, row[0], row[1], row[2], row[3]])} />
      </Card>
      <Card title="用户关注点词云">
        <div className="word-cloud">{['录取分数', '公费师范', '专业', '分数线', '招生计划', '宿舍', '优师计划', '就业', '学费', '调剂', '位次', '师范', '投档', '升学', '报名', '环境', '师资'].map((word, index) => <span style={{ fontSize: `${14 + (index % 6) * 4}px` }} key={word}>{word}</span>)}</div>
      </Card>
    </>
  );
}

function SpecialPage() {
  return (
    <>
      <StatGrid stats={specialStats} />
      <div className="grid-two">
        <Card title="师范类 vs 非师范类咨询对比"><Chart option={pieOption([{ name: '师范类', value: 66 }, { name: '非师范类', value: 34 }])} height={270} /></Card>
        <Card title="专项计划咨询量占比"><Chart option={barOption(specialPlans.map((item) => item[0] as string), specialPlans.map((item) => item[1] as number), '#3478f6')} height={270} /></Card>
      </div>
      <div className="grid-two">
        <Card title="各专业考生关注度 TOP10"><Chart option={horizontalBarOption(majorAttention)} height={340} /></Card>
        <Card title="录取规则与政策类问题统计">
          <div className="policy-bars">{['投档比例', '调剂退档规则', '同分录取规则', '单科成绩要求', '体检限制专业', '加分政策', '少数民族照顾'].map((name, index) => <div key={name}><span>{876 - index * 111}</span><b>{name}</b></div>)}</div>
        </Card>
      </div>
      <Card title="专项计划详细数据">
        <DataTable headers={['专项名称', '咨询量', '占比', '环比变化', '热度趋势']} rows={specialPlans.map((row) => [...row, '▁▃▅▆▇'])} />
      </Card>
    </>
  );
}

function EvaluationOverviewPage() {
  return (
    <>
      <PageTitle title="测评数据总览" subtitle="志愿填报工具使用分析 · 考生刚需程度洞察" />
      <div className="segment"><button>今日</button><button className="active">近7天</button><button>近30天</button><button>全年</button></div>
      <StatGrid stats={overviewStats} />
      <div className="grid-two">
        <Card title="按地域维度 · 各省份测评统计">
          <DataTable headers={['省份', '发起量', '有效提交', '转化率']} rows={provinceEvaluations} />
        </Card>
        <Card title="每日测评使用量趋势">
          <Chart option={barOption(['7/10', '7/11', '7/12', '7/13', '7/14', '7/15', '7/16', '7/17'], [8500, 9200, 11500, 12800, 15200, 18500, 22100, 25800], '#3d8bfd')} height={270} />
          <div className="chart-note"><span>峰值：25,800</span><span>高峰日：7月17日</span></div>
        </Card>
      </div>
      <div className="grid-three">
        <Card title="考生类型 · 科类分布"><ProgressList items={[['物理类', 47.4], ['历史类', 31.8], ['综合改革', 13.4], ['理工', 6.3], ['文史', 1.1]]} /></Card>
        <Card title="考生类型 · 报考类型分布"><ProgressList items={[['普通本科批', 58], ['公费师范', 20.1], ['专项计划', 13.4], ['优师计划', 8.5]]} /></Card>
        <Card title="分数段 · 位次区间分布"><ProgressList items={[['1万名以内', 9.9], ['1万-3万名', 29.9], ['3万-5万名', 36.3], ['5万名以外', 23.9]]} /></Card>
      </div>
      <Card title="深度业务洞察">
        <div className="insight-cards">
          {['师范专业热度分析', '位次匹配分析', '测评留存分析'].map((title) => <article key={title}><h3>{title}</h3><p>完成测评后，用户会再次咨询专业、学费、就业等信息，测评到深度咨询再到报考转化的链路已初步形成。</p></article>)}
        </div>
      </Card>
    </>
  );
}

function EvaluationPage() {
  const [query, setQuery] = useState('');
  const [record, setRecord] = useState<EvaluationRecord | null>(null);
  const rows = evaluationRecords.filter((item) => `${item.id}${item.province}${item.phone}`.includes(query));
  return (
    <>
      <PageTitle title="测评明细" subtitle="每条记录存档、可溯源" />
      <Toolbar>
        <SearchBox value={query} onChange={setQuery} placeholder="搜索编号、省份、联系方式..." />
        <PrimaryButton ghost><Filter size={16} />筛选</PrimaryButton>
        <PrimaryButton ghost><Download size={16} />导出Excel</PrimaryButton>
      </Toolbar>
      <Card>
        <DataTable
          headers={['编号', '测评类型', '访问省份', 'IP属地', '科类', '报考类型', '分数', '位次', '联系方式', '状态', '操作']}
          rows={rows.map((item) => [item.id, item.type, item.province, item.ip, item.subject, item.applyType, item.score, item.rank, item.phone, <StatusBadge value={item.status} />, item.status === '已完成' ? <button className="table-action" type="button" onClick={() => setRecord(item)}>查看</button> : '-'])}
        />
        <Pagination total={rows.length} />
      </Card>
      {record ? <Modal title={`测评结果 ${record.id}`} onClose={() => setRecord(null)}><DetailGrid items={record} /></Modal> : null}
    </>
  );
}

function ConversationsPage() {
  const [query, setQuery] = useState('');
  const [apiRows, setApiRows] = useState<AdminConversationListItem[] | null>(null);
  const [selected, setSelected] = useState<AdminConversationListItem | null>(null);
  const [detail, setDetail] = useState<AdminConversationDetail | null>(null);
  const [loadError, setLoadError] = useState('');
  const mockRows = conversations
    .filter((item) => item.join('').includes(query))
    .map(([id, province, updatedAt, messageCount, status, manualIntervention]) => ({
      id: id as string,
      province: province as string,
      updatedAt: updatedAt as string,
      messageCount: messageCount as number,
      status: status as string,
      manualIntervention: manualIntervention === '是',
      lastMessage: '本地样例对话',
    }));
  const rows = apiRows ?? mockRows;

  useEffect(() => {
    let alive = true;
    fetchAdminConversations(query)
      .then((data) => {
        if (alive) {
          setApiRows(data.items);
          setLoadError('');
        }
      })
      .catch((error: Error) => {
        if (alive) {
          setApiRows(null);
          setLoadError(error.message);
        }
      });
    return () => {
      alive = false;
    };
  }, [query]);

  function openConversation(row: AdminConversationListItem) {
    setSelected(row);
    setDetail(null);
    fetchAdminConversationDetail(row.id)
      .then(setDetail)
      .catch(() => setDetail(null));
  }

  return (
    <>
      <Toolbar>
        <SelectLike>近7天</SelectLike><SelectLike>全部省份</SelectLike><SelectLike>全部状态</SelectLike>
        <SearchBox value={query} onChange={setQuery} placeholder="搜索对话内容..." />
        <PrimaryButton ghost><Download size={16} />导出</PrimaryButton>
      </Toolbar>
      {loadError ? <div className="warning-box"><AlertCircle size={18} />真实对话数据暂不可用，已显示本地样例</div> : null}
      <Card title={`对话记录列表 ${rows.length} 条`}>
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
            <div className="empty-state">正在读取对话详情，或当前为本地样例数据。</div>
          )}
        </Modal>
      ) : null}
    </>
  );
}

function KnowledgePage() {
  const [query, setQuery] = useState('');
  const [items, setItems] = useState(knowledgeBase);
  const [apiItems, setApiItems] = useState<KnowledgeItem[] | null>(null);
  const [chunks, setChunks] = useState<AdminKnowledgeChunkItem[]>([]);
  const [faqTotal, setFaqTotal] = useState<number | null>(null);
  const [chunkTotal, setChunkTotal] = useState<number | null>(null);
  const [loadError, setLoadError] = useState('');
  const [modal, setModal] = useState<'new' | 'import' | null>(null);
  const rows = apiItems ?? items.filter((item) => `${item.question}${item.answer}${item.similar}`.includes(query));

  useEffect(() => {
    let alive = true;
    Promise.all([
      fetchAdminFaqs(query),
      fetchAdminKnowledgeChunks(query),
    ])
      .then(([faqList, chunkList]) => {
        if (alive) {
          setApiItems(faqList.items);
          setChunks(chunkList.items);
          setFaqTotal(faqList.total);
          setChunkTotal(chunkList.total);
          setLoadError('');
        }
      })
      .catch((error: Error) => {
        if (alive) {
          setApiItems(null);
          setChunks([]);
          setFaqTotal(null);
          setChunkTotal(null);
          setLoadError(error.message);
        }
      });
    return () => {
      alive = false;
    };
  }, [query]);

  function addUnknown(question: string) {
    setItems((current) => [...current, {
      id: current.length + 1,
      question,
      similar: '系统自动归集',
      answer: '待招生办补充标准答案。',
      source: '未知问题池',
      updatedAt: '2024-06-14',
      status: '启用',
      hits: 0,
    }]);
  }

  return (
    <>
      <Toolbar>
        <SelectLike>全部</SelectLike>
        <SearchBox value={query} onChange={setQuery} placeholder="搜索问答内容..." />
        <PrimaryButton ghost onClick={() => setModal('import')}><Upload size={16} />批量导入</PrimaryButton>
        <PrimaryButton ghost><Download size={16} />导出</PrimaryButton>
        <PrimaryButton onClick={() => setModal('new')}><Plus size={16} />本地新增预览</PrimaryButton>
      </Toolbar>
      {loadError ? <div className="warning-box"><AlertCircle size={18} />真实知识库暂不可用，已显示本地样例</div> : null}
      <Card title={`标准问答库 ${faqTotal ?? rows.length} 条`}>
        <DataTable headers={['ID', '标准问题', '相似问法', '标准答案', '来源', '更新时间', '状态', '命中']} rows={rows.map((item) => [item.id, item.question, item.similar, item.answer, item.source, item.updatedAt, <StatusBadge value={item.status} />, item.hits])} />
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
          <div className="empty-state compact">没有匹配到文档片段。</div>
        )}
      </Card>
      <Card title="未知问题归集池" action={<span className="soft-pill amber">本地样例</span>}>
        <p className="soft-note">第一阶段先接真实 FAQ 和文档片段只读数据；未覆盖问题、审核状态、命中统计会在后续接入事件日志后改为真实分析。</p>
        <div className="unknown-list">
          {unknownQuestions.map(([question, count, status]) => (
            <div key={question as string}>
              <div><b>{question}</b><span>出现 {count} 次</span><StatusBadge value={status as string} /></div>
              <PrimaryButton ghost onClick={() => addUnknown(question as string)}><Plus size={16} />加入知识库</PrimaryButton>
            </div>
          ))}
        </div>
      </Card>
      {modal ? <KnowledgeModal mode={modal} onClose={() => setModal(null)} onSave={(item) => { setItems((current) => [...current, item]); setModal(null); }} /> : null}
    </>
  );
}

function KnowledgeModal({ mode, onClose, onSave }: { mode: 'new' | 'import'; onClose: () => void; onSave: (item: KnowledgeItem) => void }) {
  const [question, setQuestion] = useState('');
  const [answer, setAnswer] = useState('');
  return (
    <Modal title={mode === 'new' ? '新增标准问答' : '批量导入问答'} onClose={onClose}>
      {mode === 'import' ? <div className="upload-drop"><Upload size={30} />拖拽 Excel 文件到此处，或点击选择文件</div> : null}
      <label className="field"><span>标准问题</span><input value={question} onChange={(event) => setQuestion(event.target.value)} placeholder="请输入标准问题" /></label>
      <label className="field"><span>标准答案</span><textarea value={answer} onChange={(event) => setAnswer(event.target.value)} placeholder="请输入标准答案" /></label>
      <PrimaryButton onClick={() => onSave({ id: Date.now(), question: question || '新增招生咨询问题', similar: '新问法', answer: answer || '请以招生办公室官方公告为准。', source: '后台新增', updatedAt: '2024-06-14', status: '启用', hits: 0 })}><Save size={16} />保存</PrimaryButton>
    </Modal>
  );
}

function TicketsPage() {
  const [active, setActive] = useState('全部');
  const [query, setQuery] = useState('');
  const [list, setList] = useState(tickets);
  const [ticket, setTicket] = useState<Ticket | null>(null);
  const filtered = list.filter((item) => (active === '全部' || item.status === active) && `${item.name}${item.content}${item.province}`.includes(query));
  const tabs = ['全部', '待处理', '处理中', '已办结'];

  function advance(id: string) {
    setList((current) => current.map((item) => item.id === id ? { ...item, status: item.status === '待处理' ? '处理中' : '已办结' } : item));
    setTicket(null);
  }

  return (
    <>
      <Toolbar>
        <div className="tabs">{tabs.map((tab) => <button className={active === tab ? 'active' : ''} type="button" onClick={() => setActive(tab)} key={tab}>{tab}</button>)}</div>
        <SearchBox value={query} onChange={setQuery} placeholder="搜索姓名/电话/内容..." />
      </Toolbar>
      <Card title={`留言工单列表 ${filtered.length} 条`}>
        <DataTable headers={['工单编号', '姓名', '省份', '咨询内容', '提交时间', '状态', '优先级', '操作']} rows={filtered.map((item) => [item.id, item.name, item.province, item.content, item.time, <StatusBadge value={item.status} />, <StatusBadge value={item.priority} />, <button className="table-action" type="button" onClick={() => setTicket(item)}>办理</button>])} />
      </Card>
      {ticket ? <Modal title={`工单 ${ticket.id}`} onClose={() => setTicket(null)}><p className="dialog-text">{ticket.content}</p><PrimaryButton onClick={() => advance(ticket.id)}><CheckCircle2 size={16} />推进状态</PrimaryButton></Modal> : null}
    </>
  );
}

function SettingsPage() {
  const [tab, setTab] = useState('数字人配置');
  const [welcome, setWelcome] = useState('您好，欢迎来到哈尔滨师范大学！我是您的招生咨询助手「沐阳」，很高兴为您服务。请问有什么可以帮助您的吗？');
  const [fallback, setFallback] = useState('抱歉，我暂时无法回答这个问题。建议您拨打招生咨询电话：0451-88060678，或者提交人工留言，我们会尽快为您解答。');
  const [saved, setSaved] = useState(false);

  return (
    <>
      <div className="tabs settings-tabs">{['数字人配置', '热门问题', '账号管理', '操作日志'].map((item) => <button className={tab === item ? 'active' : ''} type="button" onClick={() => setTab(item)} key={item}>{item}</button>)}</div>
      {tab === '数字人配置' ? (
        <div className="grid-two">
          <Card title="数字人欢迎语配置">
            <label className="field"><span>开场欢迎语</span><textarea value={welcome} onChange={(event) => setWelcome(event.target.value)} /></label>
            <label className="field"><span>兜底话术配置</span><textarea value={fallback} onChange={(event) => setFallback(event.target.value)} /></label>
            <PrimaryButton onClick={() => setSaved(true)}><Save size={16} />保存配置</PrimaryButton>
            {saved ? <span className="save-tip">配置已保存，预览已同步更新</span> : null}
          </Card>
          <Card title="配置预览">
            <div className="preview-card"><img src="/assets/muyang.gif" alt="沐阳" /><b>沐阳</b><p>{welcome}</p><small>兜底：{fallback}</small></div>
          </Card>
        </div>
      ) : (
        <Card title={tab}><div className="empty-state">该模块已按源站样式预留，可继续扩展真实配置项。</div></Card>
      )}
    </>
  );
}

function BigScreenPage() {
  const [category, setCategory] = useState('总榜');
  const now = new Date().toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }).replace(/\//g, '/');
  const bigStatIcons = [Users, MessageSquare, Globe2, LayoutDashboard, MapPinned, Target, FileText];
  const provincePercents = ['12.5%', '6.6%', '5.3%', '5.3%', '4.7%', '4.7%', '4.1%', '4.1%', '4.1%', '3.5%', '3.5%', '3.5%'];
  const provinceEvaluate = [3302, 1737, 1390, 1390, 1234, 1234, 1078, 1076, 1076, 923, 921, 921];
  const behaviorData = [
    ['今日咨询用户', '7,785', '+20.0%', [12, 18, 22, 28, 31, 29, 36]],
    ['今日咨询问答', '32,777', '+20.5%', [18, 21, 25, 29, 34, 33, 39]],
    ['今日志愿测评量', '1,168', '+45.2%', [10, 14, 16, 22, 28, 25, 31]],
    ['近7天用户', '43,252', '+17.3%', [24, 27, 25, 23, 29, 31, 34]],
    ['近7天问答', '182,094', '+12.4%', [19, 21, 20, 23, 25, 26, 30]],
    ['近7天志愿测评量', '6,488', '+52.3%', [12, 17, 15, 21, 25, 24, 33]],
  ] as const;

  const mapOption = {
    animation: false,
    backgroundColor: 'transparent',
    tooltip: { trigger: 'item' },
    visualMap: {
      min: 0,
      max: 24000,
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
      data: mapData,
    }],
  };

  return (
    <main className="big-screen">
      <header className="big-header">
        <div><b>哈师大招生智能体管理后台</b><span>{now}</span></div>
        <h1>哈尔滨师范大学 · 招生智能体<small>全国生源招生咨询态势总览</small></h1>
        <p>数据基于用户IP解析省份统计</p>
      </header>
      <div className="big-stat-row">
        {bigStats.map((stat, index) => {
          const Icon = bigStatIcons[index] ?? Target;
          return (
            <div key={stat.label}>
              <i><Icon size={20} /></i>
              <span>{stat.label}</span>
              <strong>{stat.value}</strong>
            </div>
          );
        })}
      </div>
      <section className="big-grid">
        <aside className="big-left">
          <BigPanel title="用户行为数据统计" action={<SelectLike>近30天</SelectLike>}>
            <div className="behavior-grid">
              {behaviorData.map(([label, value, delta, points], index) => (
                <div key={label}>
                  <span>{label}</span>
                  <strong>{value}<em>↑{delta.replace('+', '')}</em></strong>
                  <SparkLine points={points} tone={index % 3 === 2 ? 'orange' : index % 3 === 1 ? 'blue' : 'green'} />
                </div>
              ))}
            </div>
            <div className="wide-metrics"><div><span>人均提问次数</span><strong>3.9</strong></div><div><span>留资转化率</span><strong>10.0%</strong></div></div>
          </BigPanel>
          <BigPanel title="各省咨询数据详情" action={<button type="button" className="export-action"><Download size={15} />导出</button>}>
            <div className="province-detail">
              <div className="donut"><strong>12</strong><span>省份</span></div>
              <div className="province-table">
                <div className="province-table-head"><span>省份</span><span>咨询量</span><span>测评量</span></div>
                <ul>{mapData.map((item, index) => (
                  <li key={item.name}>
                    <span><i style={{ background: `hsl(212, 95%, ${68 - (index % 5) * 6}%)` }} />{item.name}{['上海', '北京'].includes(item.name) ? '市' : '省'}</span>
                    <b>{item.value.toLocaleString()} <small>({provincePercents[index]})</small></b>
                    <em>{provinceEvaluate[index].toLocaleString()} <small>({provincePercents[index]})</small></em>
                  </li>
                ))}</ul>
                <div className="province-scrollbar"><span /><b /></div>
              </div>
            </div>
          </BigPanel>
        </aside>
        <section className="map-area">
          <BigPanel title="全国各省咨询热力分布" action={<span>IP解析精准度≥99.9%</span>}>
            <Chart option={mapOption} height={390} />
          </BigPanel>
          <BigPanel title="实时咨询动态" action={<span className="live-dot">实时</span>}>
            <div className="live-list">
              <div className="live-track">
                {[...realtimeMessages, ...realtimeMessages].map(([province, ask, answer, time], index) => (
                  <div key={`${province}-${time}-${index}`}>
                    <b>{province}</b>
                    <span>{ask}</span>
                    <em>答：{answer}</em>
                    <time>{time}</time>
                  </div>
                ))}
              </div>
            </div>
          </BigPanel>
        </section>
        <aside className="big-right">
          <BigPanel title="全国热点问题TOP榜" action={<RefreshCw size={16} />}>
            <div className="big-tabs">{['总榜 (100%)', '院校介绍 (25%)', '分数与位次 (32%)', '招生计划 (18%)', '录取政策 (12%)', '专项与公费师范 (8%)', '就读与就业 (5%)'].map((item) => <button className={category === item.split(' ')[0] ? 'active' : ''} type="button" onClick={() => setCategory(item.split(' ')[0])} key={item}>{item}</button>)}</div>
            <div className="big-insight">
              <b>智能洞察</b>
              <p>考生咨询集中在三个方面：<mark>一是录取分数线及位次预测</mark>，说明考生对录取难度高度关注；<mark>二是师范类专业选择、就业前景及公费师范政策</mark>，表明教师职业吸引力持续增强；<mark>三是招生专业目录、选科要求和录取规则</mark>，体现考生志愿填报日趋理性。</p>
              <p>高校应重点关注<mark>分数线透明化、师范生就业数据公示、专项计划宣传</mark>等工作。</p>
            </div>
            <ol className="big-rank">{bigTopQuestions.map(([question, count, delta], index) => <li key={question}><b>{index + 1}</b><span>{question}</span><em>{count}</em><strong className={delta.startsWith('-') ? 'negative' : ''}>{delta}</strong></li>)}</ol>
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

function ProgressList({ items }: { items: Array<[string, number]> }) {
  return <div className="progress-list">{items.map(([name, value]) => <div key={name}><div><span>{name}</span><b>{value}%</b></div><i><em style={{ width: `${value}%` }} /></i></div>)}</div>;
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

function Pagination({ total }: { total: number }) {
  return <div className="pagination"><span>共 {total} 条</span><button>上一页</button><b>第 1 页</b><button>下一页</button></div>;
}

function DetailGrid({ items }: { items: Record<string, unknown> }) {
  return <div className="detail-grid">{Object.entries(items).map(([key, value]) => <div key={key}><span>{key}</span><b>{String(value)}</b></div>)}</div>;
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
