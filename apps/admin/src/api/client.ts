type ApiEnvelope<T> = {
  success: boolean;
  data?: T;
  error?: {
    code: string;
    message: string;
  } | null;
};

const DEFAULT_API_BASE = 'http://127.0.0.1:4000';

export async function apiGet<T>(path: string, params?: Record<string, string | number | undefined>): Promise<T> {
  const base = import.meta.env.VITE_ADMIN_API_BASE_URL?.trim() || DEFAULT_API_BASE;
  const url = new URL(path, base.endsWith('/') ? base : `${base}/`);
  Object.entries(params ?? {}).forEach(([key, value]) => {
    if (value !== undefined && value !== '') {
      url.searchParams.set(key, String(value));
    }
  });

  const headers: Record<string, string> = {
    Accept: 'application/json',
  };
  const token = import.meta.env.VITE_ADMIN_API_TOKEN?.trim();
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }

  const response = await fetch(url, { headers });
  const envelope = (await response.json()) as ApiEnvelope<T>;
  if (!response.ok || !envelope.success || envelope.data === undefined) {
    throw new Error(envelope.error?.message || `请求失败：${response.status}`);
  }
  return envelope.data;
}
