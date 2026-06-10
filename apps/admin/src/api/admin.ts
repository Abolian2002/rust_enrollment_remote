import { apiGet } from './client';
import type {
  AdminConversationDetail,
  AdminConversationList,
  AdminDashboardSnapshot,
  AdminFaqList,
  AdminKnowledgeChunkList,
} from '../types/admin';

export function fetchAdminDashboard() {
  return apiGet<AdminDashboardSnapshot>('/api/v1/admin/dashboard/summary');
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

export function fetchAdminKnowledgeChunks(q: string) {
  return apiGet<AdminKnowledgeChunkList>('/api/v1/admin/knowledge/chunks', {
    q,
    page: 1,
    pageSize: 10,
  });
}
