import { apiGet, apiPatch, apiPost } from './client';
import type {
  AdminAuditLogList,
  AdminAdmissionsAnalyticsSnapshot,
  AdminBigScreenSnapshot,
  AdminConversationDetail,
  AdminConversationList,
  AdminDashboardSnapshot,
  AdminFeedbackItem,
  AdminFeedbackType,
  AdminFaqList,
  AdminInsightsSnapshot,
  AdminKnowledgeChunkList,
  AdminKnowledgeCoverageSnapshot,
  AdminSettings,
  AdminSpecialSnapshot,
  AdminTicketItem,
  AdminTicketList,
} from '../types/admin';

export function fetchAdminDashboard() {
  return apiGet<AdminDashboardSnapshot>('/api/v1/admin/dashboard/summary');
}

export function fetchAdminInsights() {
  return apiGet<AdminInsightsSnapshot>('/api/v1/admin/analytics/insights');
}

export function fetchAdminSpecial() {
  return apiGet<AdminSpecialSnapshot>('/api/v1/admin/analytics/special');
}

export function fetchAdminAdmissionsAnalytics() {
  return apiGet<AdminAdmissionsAnalyticsSnapshot>('/api/v1/admin/analytics/admissions');
}

export function fetchAdminKnowledgeCoverage() {
  return apiGet<AdminKnowledgeCoverageSnapshot>('/api/v1/admin/analytics/knowledge');
}

export function fetchAdminBigScreen() {
  return apiGet<AdminBigScreenSnapshot>('/api/v1/admin/analytics/big-screen');
}

export function fetchAdminConversations(q: string) {
  return apiGet<AdminConversationList>('/api/v1/admin/conversations', {
    q,
    page: 1,
    pageSize: 50,
  });
}

export function fetchAdminConversationDetail(id: string) {
  return apiGet<AdminConversationDetail>(`/api/v1/admin/conversations/${encodeURIComponent(id)}`);
}

export function fetchAdminFaqs(q: string) {
  return apiGet<AdminFaqList>('/api/v1/admin/knowledge/faqs', {
    q,
    page: 1,
    pageSize: 80,
  });
}

export function createAdminFaq(payload: {
  question: string;
  answer: string;
  category?: string;
  tags?: string[];
  status?: 'draft' | 'published';
  sourceLabel?: string;
}) {
  return apiPost<AdminFaqList['items'][number]>('/api/v1/admin/knowledge/faqs', payload);
}

export function updateAdminFaq(id: string, payload: {
  question?: string;
  answer?: string;
  category?: string;
  tags?: string[];
  status?: 'draft' | 'published';
  sourceLabel?: string;
}) {
  return apiPatch<AdminFaqList['items'][number]>(`/api/v1/admin/knowledge/faqs/${encodeURIComponent(id)}`, payload);
}

export function fetchAdminKnowledgeChunks(q: string) {
  return apiGet<AdminKnowledgeChunkList>('/api/v1/admin/knowledge/chunks', {
    q,
    page: 1,
    pageSize: 10,
  });
}

export function createAdminFeedback(payload: {
  conversationId?: string;
  messageId?: string;
  feedbackType: AdminFeedbackType;
  comment?: string;
  handledBy?: string;
  status?: 'open' | 'resolved';
}) {
  return apiPost<AdminFeedbackItem>('/api/v1/admin/feedback', payload);
}

export function fetchAdminTickets(q: string, status: string) {
  return apiGet<AdminTicketList>('/api/v1/admin/tickets', {
    q,
    status: status === '全部' ? undefined : status,
    page: 1,
    pageSize: 80,
  });
}

export function updateAdminTicket(id: string, payload: {
  status?: AdminTicketItem['status'];
  resolution?: string;
  handledBy?: string;
}) {
  return apiPatch<AdminTicketItem>(`/api/v1/admin/tickets/${encodeURIComponent(id)}`, payload);
}

export function fetchAdminSettings() {
  return apiGet<AdminSettings>('/api/v1/admin/settings');
}

export function updateAdminSettings(payload: AdminSettings & { updatedBy?: string }) {
  return apiPatch<AdminSettings>('/api/v1/admin/settings', payload);
}

export function fetchAdminAuditLogs() {
  return apiGet<AdminAuditLogList>('/api/v1/admin/audit-logs', {
    page: 1,
    pageSize: 50,
  });
}
