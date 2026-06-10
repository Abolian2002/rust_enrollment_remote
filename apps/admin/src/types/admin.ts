import type { KnowledgeItem, Stat } from '../data/mock';

export type AdminChartDatum = {
  name: string;
  value: number;
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
