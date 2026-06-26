type ApiSuccess<T> = {
  success: true;
  data: T;
  meta: Record<string, unknown>;
  error: null;
};

type ApiFailure = {
  success: false;
  data: null;
  meta: Record<string, unknown>;
  error: {
    code: string;
    message: string;
  };
};

type ApiEnvelope<T> = ApiSuccess<T> | ApiFailure;

export type ChatProfileInput = {
  province?: string;
  score?: number;
  rank?: number;
  subjectType?: string;
};

export type ChatRequestInput = {
  conversationId?: string;
  message: string;
  profile?: ChatProfileInput;
  model?: string;
};

export type ChatCitation = {
  year?: number;
  sourceLabel: string;
  sourceUrl?: string | null;
};

export type ChatStructuredResult = {
  type: string;
  [key: string]: unknown;
};

export type ChatResult = {
  conversationId: string;
  reply: string;
  structuredResult: ChatStructuredResult;
  citations: ChatCitation[];
};

export type PublicTicketInput = {
  name?: string;
  province: string;
  phone: string;
  email?: string;
  content: string;
};

export type PublicTicketResult = {
  id: string;
  name: string;
  phone?: string;
  email?: string;
  province: string;
  content: string;
  status: string;
  priority: string;
  createdAt: string;
};

export type ChatStreamStatus = "resolving" | "retrieving" | "generating";

export type ChatStreamHandlers = {
  onStatus?: (status: ChatStreamStatus) => void;
  onChunk?: (delta: string) => void;
  onAudioChunk?: (chunk: ArrayBuffer) => void;
  onAudioDone?: () => void;
};

let activeVoiceSocket: WebSocket | null = null;

export function stopActiveVoiceStream() {
  if (!activeVoiceSocket) {
    return;
  }
  try {
    activeVoiceSocket.close();
  } catch {
    // Ignore close errors from already-closed sockets.
  } finally {
    activeVoiceSocket = null;
  }
}

function isChatStreamStatus(value: unknown): value is ChatStreamStatus {
  return value === "resolving" || value === "retrieving" || value === "generating";
}

function getApiBaseUrl() {
  const baseUrl = process.env.NEXT_PUBLIC_API_BASE_URL?.trim();
  // Keep build/dev predictable even if env is missing.
  return baseUrl && baseUrl.length > 0 ? baseUrl : "http://localhost:4000";
}

function buildApiUrl(pathname: string, params?: Record<string, string | number | undefined>) {
  const url = new URL(pathname, getApiBaseUrl());
  if (params) {
    for (const [key, value] of Object.entries(params)) {
      if (value === undefined) continue;
      url.searchParams.set(key, String(value));
    }
  }
  return url.toString();
}

function buildApiWsUrl(pathname: string) {
  const url = new URL(pathname, getApiBaseUrl());
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

async function apiGet<T>(pathname: string, params?: Record<string, string | number | undefined>) {
  const response = await fetch(buildApiUrl(pathname, params), {
    // Query pages should always reflect latest DB state; also avoids build-time prefetching assumptions.
    cache: "no-store"
  });

  if (!response.ok) {
    const text = await response.text().catch(() => "");
    throw new Error(
      `API request failed: ${response.status} ${response.statusText}${text ? `; body=${text}` : ""}`
    );
  }

  const payload = (await response.json()) as ApiEnvelope<T>;
  if (!payload.success) {
    throw new Error(`API error: ${payload.error.code}: ${payload.error.message}`);
  }
  return payload.data;
}

async function apiPost<T>(pathname: string, body: unknown) {
  const response = await fetch(buildApiUrl(pathname), {
    method: "POST",
    headers: {
      "Content-Type": "application/json"
    },
    body: JSON.stringify(body),
    cache: "no-store"
  });

  if (!response.ok) {
    const text = await response.text().catch(() => "");
    throw new Error(
      `API request failed: ${response.status} ${response.statusText}${text ? `; body=${text}` : ""}`
    );
  }

  const payload = (await response.json()) as ApiEnvelope<T>;
  if (!payload.success) {
    throw new Error(`API error: ${payload.error.code}: ${payload.error.message}`);
  }
  return payload.data;
}

function parseEventBlock(block: string) {
  const lines = block
    .split("\n")
    .map((line) => line.trimEnd())
    .filter(Boolean);

  let eventName = "message";
  const dataLines: string[] = [];

  for (const line of lines) {
    if (line.startsWith("event:")) {
      eventName = line.slice("event:".length).trim();
      continue;
    }

    if (line.startsWith("data:")) {
      dataLines.push(line.slice("data:".length).trim());
    }
  }

  return {
    eventName,
    data: dataLines.join("\n")
  };
}

function yieldToBrowser() {
  return new Promise<void>((resolve) => {
    window.setTimeout(resolve, 0);
  });
}

export type MajorCatalogItem = {
  id: string;
  slug: string;
  code: string;
  name: string;
  degreeLevel: string | null;
  durationYears: number | null;
  tuitionFee: number | null;
  isNormalMajor: boolean;
  hasMaster: boolean;
  hasDoctor: boolean;
  university: { code?: string; name: string };
  latestScore: { year: number; minScore: number } | null;
  tags: string[];
};

export type MajorDetail = {
  id: string;
  slug: string;
  code: string;
  name: string;
  degreeLevel: string | null;
  durationYears: number | null;
  tuitionFee: number | null;
  isNormalMajor: boolean;
  hasMaster: boolean;
  hasDoctor: boolean;
  introduction: string | null;
  employmentSummary: string | null;
  postgraduateSummary: string | null;
  university: { code: string; name: string };
  scoreTrend: Array<{ year: number; minScore: number }>;
  planTrend: Array<{ year: number; plannedCount: number }>;
};

export type AdmissionScoreItem = {
  id: string;
  year: number;
  batch: string;
  subjectType: string;
  admittedCount: number | null;
  minScore: number;
  avgScore: number | null;
  maxScore: number | null;
  minRank: number | null;
  avgRank: number | null;
  maxRank: number | null;
  sourceLabel: string | null;
  sourceUrl: string | null;
  dataVersion: string | null;
};

export type AdmissionPlanItem = {
  id: string;
  year: number;
  batch: string;
  subjectType: string;
  plannedCount: number;
  actualCount: number | null;
  province: { code: string; name: string } | null;
  sourceLabel: string | null;
  dataVersion: string | null;
};

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

export type AdminImportBatchSummary = {
  totalRows?: number;
  acceptedRows?: number;
  rejectedRows?: number;
  rowErrors?: Array<{
    rowIndex: number;
    messages: string[];
  }>;
  rolledBackRows?: number;
  rolledBackAt?: string;
  error?: string;
};

export type AdminImportBatch = {
  id: string;
  batchNo: string;
  importType: string;
  sourceFileName: string;
  sourceLabel?: string | null;
  sourceHash: string;
  dataVersion: string;
  status: string;
  importedBy: string | null;
  summary: AdminImportBatchSummary | null;
  notes: string | null;
  isSample: boolean;
  createdAt: string;
  updatedAt: string;
};

export type AdmissionScoreImportRowInput = {
  universityId?: string;
  universityCode?: string;
  majorId?: string;
  majorSlug?: string;
  provinceCode: string;
  year: number;
  batch: string;
  subjectType: string;
  minScore: number;
  avgScore: number;
  maxScore?: number | null;
  minRank?: number | null;
  avgRank?: number | null;
  maxRank?: number | null;
  sourceUrl?: string | null;
};

export type AdmissionScoreImportPayload = {
  sourceFileName: string;
  sourceLabel: string;
  dataVersion: string;
  importedBy: string;
  dryRun?: boolean;
  rows: AdmissionScoreImportRowInput[];
};

export type AdmissionScoreImportResult = {
  batchId: string | null;
  persisted: boolean;
  status: "preview" | "completed";
  summary: {
    totalRows: number;
    acceptedRows: number;
    rejectedRows: number;
  };
  rowErrors: Array<{
    rowIndex: number;
    messages: string[];
  }>;
  acceptedPreviewRows: Array<{
    rowIndex: number;
    universityId: string;
    majorId: string;
    provinceId: string;
    year: number;
    batch: string;
    subjectType: string;
    dataVersion: string;
  }>;
  batch?: AdminImportBatch;
};

export type AdmissionScoreRollbackResult = {
  batch: AdminImportBatch;
  rolledBackRows: number;
};

export type AdminFaqStatus = "draft" | "published";

export type AdminFaq = {
  id: string;
  question: string;
  answer: string;
  category: string;
  tags: string[];
  status: AdminFaqStatus;
  sourceLabel: string;
  dataVersion?: string;
  createdAt?: string;
  updatedAt: string;
};

export type AdminFaqInput = {
  question: string;
  answer: string;
  category: string;
  tags: string[];
  status: AdminFaqStatus;
  sourceLabel: string;
};

export type AdminPolicyStatus = "active" | "inactive";

export type AdminPolicy = {
  id: string;
  title: string;
  category: string;
  year: number | null;
  sourceLabel?: string;
  sourceUrl: string | null;
  contentText: string;
  publishedAt: string | null;
  status: AdminPolicyStatus;
  dataVersion?: string;
  createdAt?: string;
  updatedAt?: string;
};

export type AdminPolicyInput = {
  title: string;
  category: string;
  year?: number | null;
  sourceUrl?: string | null;
  contentText: string;
  publishedAt?: string | null;
  status: AdminPolicyStatus;
};

export type AdminConversationStatus = "open" | "resolved";

export type AdminConversation = {
  id: string;
  sessionKey: string;
  provinceCode: string | null;
  score: number | null;
  rank: number | null;
  subjectType: string | null;
  interestTags: string[];
  intendedMajors: string[];
  createdAt: string;
  updatedAt: string;
  summary?: string;
  lastMessage: string;
  latestMessage?: {
    id: string;
    role: string;
    content: string;
    createdAt: string;
  } | null;
  feedbackSummary?: {
    total: number;
    open: number;
    resolved: number;
    incorrect?: number;
    helpful?: number;
    manualFix?: number;
  };
  status: AdminConversationStatus;
};

export type AdminFeedbackType = "incorrect" | "helpful" | "manual-fix";

export type AdminConversationCorrectionDraft = {
  feedbackType: AdminFeedbackType;
  note: string;
  resolution: string;
};

export async function listMajors(params: { q?: string } = {}) {
  return apiGet<MajorCatalogItem[]>("/api/v1/majors", params.q ? { q: params.q } : undefined);
}

export async function getMajorBySlug(slug: string) {
  return apiGet<MajorDetail>(`/api/v1/majors/${encodeURIComponent(slug)}`);
}

export async function listAdmissionScores(params: {
  province: string;
  majorSlug: string;
  year?: number;
  subjectType?: string;
}) {
  return apiGet<AdmissionScoreItem[]>("/api/v1/admission/scores", params);
}

export async function listAdmissionPlansByMajor(params: {
  majorSlug: string;
  year?: number;
  subjectType?: string;
}) {
  return apiGet<AdmissionPlanItem[]>("/api/v1/admission/plans/by-major", params);
}

export async function sendChatMessage(input: ChatRequestInput) {
  return apiPost<ChatResult>("/api/v1/chat", input);
}

export async function createPublicTicket(input: PublicTicketInput) {
  return apiPost<PublicTicketResult>("/api/v1/tickets", input);
}

export async function streamChatMessage(input: ChatRequestInput, handlers: ChatStreamHandlers = {}) {
  let currentStatus: ChatStreamStatus | null = null;
  const emitStatus = (status: ChatStreamStatus) => {
    if (currentStatus === status) {
      return;
    }
    currentStatus = status;
    handlers.onStatus?.(status);
  };

  emitStatus("resolving");

  const response = await fetch(buildApiUrl("/api/v1/chat/stream"), {
    method: "POST",
    headers: {
      "Content-Type": "application/json"
    },
    body: JSON.stringify(input),
    cache: "no-store"
  });

  if (!response.ok) {
    const text = await response.text().catch(() => "");
    throw new Error(
      `API request failed: ${response.status} ${response.statusText}${text ? `; body=${text}` : ""}`
    );
  }

  if (!response.body) {
    throw new Error("API request failed: streaming response body is missing");
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let result: ChatResult | null = null;
  let emittedGenerating = false;
  let chunksSinceYield = 0;

  while (true) {
    const { value, done } = await reader.read();
    buffer += decoder.decode(value, { stream: !done });

    const blocks = buffer.split("\n\n");
    buffer = blocks.pop() ?? "";

    for (const block of blocks) {
      const trimmedBlock = block.trim();
      if (!trimmedBlock) {
        continue;
      }

      const { eventName, data } = parseEventBlock(trimmedBlock);

      if (eventName === "status") {
        const payload = JSON.parse(data) as { status?: unknown };
        if (isChatStreamStatus(payload.status)) {
          emitStatus(payload.status);
        }
        continue;
      }

      if (eventName === "chunk") {
        const payload = JSON.parse(data) as { delta?: string };
        if (!emittedGenerating) {
          emittedGenerating = true;
          emitStatus("generating");
        }
        if (payload.delta) {
          handlers.onChunk?.(payload.delta);
          chunksSinceYield += 1;
          if (chunksSinceYield >= 4) {
            chunksSinceYield = 0;
            await yieldToBrowser();
          }
        }
        continue;
      }

      if (eventName === "message") {
        const payload = JSON.parse(data) as ApiEnvelope<ChatResult>;
        if (!payload.success) {
          throw new Error(`API error: ${payload.error.code}: ${payload.error.message}`);
        }
        result = payload.data;
        continue;
      }

      if (eventName === "done") {
        break;
      }
    }

    if (done) {
      break;
    }
  }

  if (!result) {
    throw new Error("API request failed: chat stream completed without a final message");
  }

  return result;
}

export async function streamVoiceChatMessage(
  input: ChatRequestInput,
  handlers: ChatStreamHandlers = {}
) {
  let currentStatus: ChatStreamStatus | null = null;
  const emitStatus = (status: ChatStreamStatus) => {
    if (currentStatus === status) {
      return;
    }
    currentStatus = status;
    handlers.onStatus?.(status);
  };

  emitStatus("resolving");

  return new Promise<ChatResult>((resolve, reject) => {
    stopActiveVoiceStream();
    const socket = new WebSocket(buildApiWsUrl("/api/v1/chat/voice"));
    activeVoiceSocket = socket;
    socket.binaryType = "arraybuffer";

    let result: ChatResult | null = null;
    let emittedGenerating = false;
    let settled = false;
    let chunksSinceYield = 0;

    const failOnce = (error: Error) => {
      if (settled) {
        return;
      }
      settled = true;
      if (activeVoiceSocket === socket) {
        activeVoiceSocket = null;
      }
      try {
        socket.close();
      } catch {
        // Ignore close errors from already-closed sockets.
      }
      reject(error);
    };

    socket.onopen = () => {
      socket.send(JSON.stringify(input));
    };

    socket.onerror = () => {
      failOnce(new Error("API request failed: voice stream websocket error"));
    };

    socket.onclose = () => {
      if (activeVoiceSocket === socket) {
        activeVoiceSocket = null;
      }
      if (!settled && !result) {
        failOnce(new Error("API request failed: voice stream closed before final message"));
      }
    };

    socket.onmessage = (event) => {
      if (event.data instanceof ArrayBuffer) {
        handlers.onAudioChunk?.(event.data);
        return;
      }

      if (event.data instanceof Blob) {
        void event.data.arrayBuffer().then((chunk) => {
          handlers.onAudioChunk?.(chunk);
        });
        return;
      }

      if (typeof event.data !== "string") {
        return;
      }

      let payload: Record<string, unknown>;
      try {
        payload = JSON.parse(event.data) as Record<string, unknown>;
      } catch {
        return;
      }

      if (payload.event === "status") {
        if (isChatStreamStatus(payload.status)) {
          emitStatus(payload.status);
        }
        return;
      }

      if (payload.event === "chunk") {
        if (!emittedGenerating) {
          emittedGenerating = true;
          emitStatus("generating");
        }
        if (typeof payload.delta === "string" && payload.delta) {
          handlers.onChunk?.(payload.delta);
          chunksSinceYield += 1;
          if (chunksSinceYield >= 4) {
            chunksSinceYield = 0;
            void yieldToBrowser();
          }
        }
        return;
      }

      if (payload.event === "message") {
        const envelope = payload.payload as ApiEnvelope<ChatResult>;
        if (!envelope?.success) {
          const error = envelope?.error;
          failOnce(
            new Error(
              `API error: ${error?.code ?? "VOICE_STREAM_ERROR"}: ${
                error?.message ?? "voice stream returned an error"
              }`
            )
          );
          return;
        }
        result = envelope.data;
        if (!settled) {
          void yieldToBrowser().then(() => {
            if (!settled && result) {
              settled = true;
              resolve(result);
            }
          });
        }
        return;
      }

      if (payload.event === "error") {
        failOnce(
          new Error(
            `API error: ${String(payload.code ?? "VOICE_STREAM_ERROR")}: ${String(
              payload.message ?? "voice stream returned an error"
            )}`
          )
        );
        return;
      }

      if (payload.event === "tts_error") {
        return;
      }

      if (payload.event === "done") {
        if (activeVoiceSocket === socket) {
          activeVoiceSocket = null;
        }
        if (!result && !settled) {
          failOnce(new Error("API request failed: voice stream completed without a final message"));
          return;
        }
        handlers.onAudioDone?.();
        socket.close();
      }
    };
  });
}

export async function listAdminImportBatches() {
  const payload = await apiGet<{ items: AdminImportBatch[] }>("/api/v1/admin/import/batches");
  return payload.items;
}

export async function importAdmissionScores(payload: AdmissionScoreImportPayload) {
  return apiPost<AdmissionScoreImportResult>("/api/v1/admin/import/admission-scores", payload);
}

export async function rollbackAdmissionScoreImportBatch(batchId: string) {
  return apiPost<AdmissionScoreRollbackResult>(
    `/api/v1/admin/import/batches/${encodeURIComponent(batchId)}/rollback`,
    {}
  );
}

export async function listAdminFaq(params: { category?: string; q?: string } = {}) {
  return apiGet<AdminFaq[]>("/api/v1/admin/faq", {
    ...(params.category ? { category: params.category } : {}),
    ...(params.q ? { q: params.q } : {})
  });
}

export async function createAdminFaq(payload: AdminFaqInput) {
  return apiPost<AdminFaq>("/api/v1/admin/faq", payload);
}

async function apiPut<T>(pathname: string, body: unknown) {
  const response = await fetch(buildApiUrl(pathname), {
    method: "PUT",
    headers: {
      "Content-Type": "application/json"
    },
    body: JSON.stringify(body),
    cache: "no-store"
  });

  if (!response.ok) {
    const text = await response.text().catch(() => "");
    throw new Error(
      `API request failed: ${response.status} ${response.statusText}${text ? `; body=${text}` : ""}`
    );
  }

  const payload = (await response.json()) as ApiEnvelope<T>;
  if (!payload.success) {
    throw new Error(`API error: ${payload.error.code}: ${payload.error.message}`);
  }
  return payload.data;
}

export async function updateAdminFaq(id: string, payload: Partial<AdminFaqInput>) {
  return apiPut<AdminFaq>(`/api/v1/admin/faq/${encodeURIComponent(id)}`, payload);
}

export async function listAdminPolicies(params: { category?: string; year?: number } = {}) {
  return apiGet<AdminPolicy[]>("/api/v1/admin/policies", {
    ...(params.category ? { category: params.category } : {}),
    ...(params.year !== undefined ? { year: params.year } : {})
  });
}

export async function createAdminPolicy(payload: AdminPolicyInput) {
  return apiPost<AdminPolicy>("/api/v1/admin/policies", payload);
}

export async function updateAdminPolicy(id: string, payload: Partial<AdminPolicyInput>) {
  return apiPut<AdminPolicy>(`/api/v1/admin/policies/${encodeURIComponent(id)}`, payload);
}

export async function listAdminConversations(params: {
  provinceCode?: string;
  status?: AdminConversationStatus;
} = {}) {
  return apiGet<AdminConversation[]>("/api/v1/admin/conversations", {
    ...(params.provinceCode ? { provinceCode: params.provinceCode } : {}),
    ...(params.status ? { status: params.status } : {})
  });
}

export async function createAdminFeedback(payload: {
  conversationId?: string;
  messageId?: string;
  feedbackType: AdminFeedbackType;
  comment?: string;
  handledById?: string;
  status?: AdminConversationStatus;
}) {
  return apiPost<{
    id: string;
    conversationId: string | null;
    messageId: string | null;
    feedbackType: AdminFeedbackType;
    comment: string | null;
    handledById: string | null;
    status: AdminConversationStatus;
    createdAt: string;
  }>("/api/v1/admin/feedback", payload);
}

export interface PublicSettings {
  welcome_message: string;
  fallback_message: string;
}

export async function fetchPublicSettings() {
  return apiGet<PublicSettings>("/api/v1/settings/public");
}
