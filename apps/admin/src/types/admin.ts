export type Stat = {
  label: string;
  value: string;
  delta?: string;
  tone?: 'blue' | 'green' | 'cyan' | 'amber' | 'red' | 'purple';
};

export type KnowledgeItem = {
  id: string | number;
  question: string;
  similar: string;
  answer: string;
  source: string;
  updatedAt: string;
  status: '启用' | '禁用';
  hits: number;
};

export type AdminChartDatum = {
  name: string;
  value: number;
};

export type AdminTopQuestion = {
  question: string;
  category: string;
  count: number;
  share: string;
};

export type AdminRealtimeMessage = {
  province: string;
  question: string;
  answer: string;
  time: string;
};

export type AdminBehaviorCard = {
  label: string;
  value: string;
  delta: string;
  points: number[];
};

export type AdminDashboardSnapshot = {
  updatedAt: string;
  stats: Stat[];
  trendDays: string[];
  trendValues: number[];
  hourlyValues: number[];
  hotQuestions: Array<[string, string]>;
  categoryStats: AdminChartDatum[];
  provinceBars: Array<[string, number]>;
};

export type AdminInsightsSnapshot = {
  updatedAt: string;
  stats: Stat[];
  categoryStats: AdminChartDatum[];
  provinceBars: Array<[string, number]>;
  topQuestions: AdminTopQuestion[];
  wordCloud: AdminChartDatum[];
  summary: string;
};

export type AdminSpecialSnapshot = {
  updatedAt: string;
  stats: Stat[];
  normalVsNonNormal: AdminChartDatum[];
  specialPlans: Array<[string, number, string, string]>;
  majorAttention: Array<[string, number]>;
  policyStats: Array<[string, number]>;
};

export type AdminAdmissionsAnalyticsSnapshot = {
  updatedAt: string;
  stats: Stat[];
  yearCounts: AdminChartDatum[];
  provinceCoverage: Array<[string, number]>;
  subjectDistribution: AdminChartDatum[];
  topMajors: Array<[string, number]>;
};

export type AdminKnowledgeCoverageSnapshot = {
  updatedAt: string;
  stats: Stat[];
  documentKinds: AdminChartDatum[];
  collegeChunks: Array<[string, number]>;
  faqCategories: AdminChartDatum[];
  policyYears: AdminChartDatum[];
};

export type AdminBigScreenSnapshot = {
  updatedAt: string;
  bigStats: Stat[];
  mapData: AdminChartDatum[];
  realtimeMessages: AdminRealtimeMessage[];
  topQuestions: AdminTopQuestion[];
  behaviorCards: AdminBehaviorCard[];
  insight: string;
};

export type AdminConversationListItem = {
  id: string;
  province: string;
  updatedAt: string;
  messageCount: number;
  status: string;
  manualIntervention: boolean;
  lastMessage: string;
};

export type AdminConversationList = {
  items: AdminConversationListItem[];
  total: number;
  page: number;
  pageSize: number;
};

export type AdminConversationMessage = {
  role: string;
  content: string;
  createdAt?: string;
};

export type AdminConversationDetail = {
  id: string;
  province: string;
  status: string;
  manualIntervention: boolean;
  messageCount: number;
  messages: AdminConversationMessage[];
};

export type AdminFaqList = {
  items: KnowledgeItem[];
  total: number;
  page: number;
  pageSize: number;
};

export type AdminKnowledgeChunkItem = {
  id: string;
  title?: string;
  excerpt: string;
  documentKind?: string;
  college?: string;
  majorName?: string;
  sourceType: string;
  updatedAt: string;
};

export type AdminKnowledgeChunkList = {
  items: AdminKnowledgeChunkItem[];
  total: number;
  page: number;
  pageSize: number;
};

export type AdminFeedbackType = 'incorrect' | 'helpful' | 'manual-fix';

export type AdminFeedbackItem = {
  id: string;
  conversationId?: string;
  messageId?: string;
  feedbackType: AdminFeedbackType;
  comment?: string;
  handledBy?: string;
  status: 'open' | 'resolved';
  createdAt: string;
};

export type AdminTicketItem = {
  id: string;
  name: string;
  phone?: string;
  email?: string;
  province: string;
  content: string;
  status: '待处理' | '处理中' | '已办结' | '已关闭';
  priority: '高' | '中' | '低';
  createdAt: string;
  updatedAt?: string;
  handledBy?: string;
  resolution?: string;
};

export type AdminTicketList = {
  items: AdminTicketItem[];
  total: number;
  page: number;
  pageSize: number;
};

export type AdminSettings = {
  welcomeMessage: string;
  fallbackMessage: string;
  updatedAt?: string;
};

export type AdminAuditLogItem = {
  id: string;
  action: string;
  targetType: string;
  targetId?: string;
  actor: string;
  detail: Record<string, unknown>;
  createdAt: string;
};

export type AdminAuditLogList = {
  items: AdminAuditLogItem[];
  total: number;
  page: number;
  pageSize: number;
};

export type AdminEvaluationSummarySnapshot = {
  updatedAt: string;
  stats: Stat[];
  provinceBars: Array<[string, number]>;
  dailyTrend: Array<[string, number]>;
  subjectDistribution: AdminChartDatum[];
  scoreDistribution: AdminChartDatum[];
  topMajors: Array<[string, number]>;
};

export type AdminEvaluationListItem = {
  id: string;
  conversationId: string;
  province: string;
  subjectType: string;
  score: number;
  rank: number;
  majorName: string;
  probability: number;
  level: string;
  summary: string;
  createdAt: string;
};

export type AdminEvaluationList = {
  items: AdminEvaluationListItem[];
  total: number;
};
