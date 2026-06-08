"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import {
  type ChatRequestInput,
  type ChatResult,
  streamChatMessage,
  type ChatCitation,
  type ChatProfileInput,
  type ChatStructuredResult,
  type ChatStreamHandlers,
  type ChatStreamStatus
} from "@/lib/api-client";

export type ChatMessage = {
  role: "user" | "assistant";
  content: string;
  meta?: string;
};

export type ChatSessionCallbacks = {
  onStreamChunk?: (delta: string) => void;
  onStreamComplete?: () => void | Promise<void>;
  onStreamError?: () => void | Promise<void>;
  onStreamStart?: () => void | Promise<void>;
  onReplyResolved?: (reply: string) => void;
  streamMessage?: (input: ChatRequestInput, handlers: ChatStreamHandlers) => Promise<ChatResult>;
};

const STORAGE_KEY = "hnu.enrollment.chat.conversationId";

const INITIAL_ASSISTANT_MESSAGE: ChatMessage = {
  role: "assistant",
  content:
    "欢迎使用哈尔滨师范大学招生信息服务平台。你可以直接告诉我省份、分数、位次和意向专业，我会先帮你判断信息是否完整，再给出建议与下一步追问。",
  meta: "招生助手"
};

function safeGetLocalStorageItem(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function safeSetLocalStorageItem(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Ignore localStorage failures in restricted environments.
  }
}

function safeRemoveLocalStorageItem(key: string) {
  try {
    window.localStorage.removeItem(key);
  } catch {
    // Ignore localStorage failures in restricted environments.
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function toNumber(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }

  if (typeof value === "string" && value.trim().length > 0) {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }

  return null;
}

function toString(value: unknown): string | null {
  if (typeof value === "string") {
    return value;
  }

  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }

  return null;
}

function getProvinceValue(value: unknown): string | undefined {
  if (typeof value === "string" && value.trim()) {
    return value;
  }

  if (isRecord(value)) {
    return toString(value.code) ?? toString(value.name) ?? undefined;
  }

  return undefined;
}

function normalizeProfileRecord(value: unknown): ChatProfileInput | undefined {
  if (!isRecord(value)) {
    return undefined;
  }

  const province = getProvinceValue(value.province);
  const score = toNumber(value.score) ?? undefined;
  const rank = toNumber(value.rank) ?? undefined;
  const subjectType = toString(value.subjectType) ?? undefined;

  const profile: ChatProfileInput = {
    ...(province ? { province } : {}),
    ...(score !== undefined ? { score } : {}),
    ...(rank !== undefined ? { rank } : {}),
    ...(subjectType ? { subjectType } : {})
  };

  return Object.keys(profile).length > 0 ? profile : undefined;
}

function chooseDisplayReply(finalReply: string, streamedReply: string) {
  return streamedReply.trim() ? streamedReply : finalReply;
}

export function extractProfile(structuredResult: ChatStructuredResult | null): ChatProfileInput | undefined {
  if (!structuredResult) {
    return undefined;
  }

  if (structuredResult.type === "follow_up") {
    return normalizeProfileRecord(structuredResult.collectedProfile);
  }

  if (structuredResult.type === "probability_assessment" && isRecord(structuredResult.assessment)) {
    const assessment = structuredResult.assessment;
    return normalizeProfileRecord({
      province: assessment.province,
      score: assessment.score,
      rank: assessment.rank,
      subjectType: assessment.subjectType
    });
  }

  if (structuredResult.type === "general_answer") {
    return normalizeProfileRecord(structuredResult.collectedProfile);
  }

  return undefined;
}

export function useChatSession(callbacks: ChatSessionCallbacks = {}) {
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [streamStatus, setStreamStatus] = useState<ChatStreamStatus | "idle">("idle");
  const [streamReply, setStreamReply] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [conversationId, setConversationId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([INITIAL_ASSISTANT_MESSAGE]);
  const [structuredResult, setStructuredResult] = useState<ChatStructuredResult | null>(null);
  const [citations, setCitations] = useState<ChatCitation[]>([]);
  const requestSeq = useRef(0);
  const bootstrappedFromUrl = useRef(false);

  useEffect(() => {
    const searchParams = new URLSearchParams(window.location.search);
    if (searchParams.get("new") === "1" || searchParams.get("autosend") === "1") {
      safeRemoveLocalStorageItem(STORAGE_KEY);
      setConversationId(null);
      return;
    }

    const storedConversationId = safeGetLocalStorageItem(STORAGE_KEY);
    if (storedConversationId?.trim()) {
      setConversationId(storedConversationId);
    }
  }, []);

  const resetSession = useCallback(() => {
    requestSeq.current += 1;
    safeRemoveLocalStorageItem(STORAGE_KEY);
    setConversationId(null);
    setInput("");
    setLoading(false);
    setStreamStatus("idle");
    setStreamReply("");
    setError(null);
    setStructuredResult(null);
    setCitations([]);
    setMessages([INITIAL_ASSISTANT_MESSAGE]);
  }, []);

  const submitMessage = useCallback(
    async (rawMessage: string) => {
      const message = rawMessage.trim();
      if (!message) {
        return;
      }

      const seq = requestSeq.current + 1;
      requestSeq.current = seq;

      setInput("");
      setLoading(true);
      setStreamStatus("retrieving");
      setStreamReply("");
      setError(null);
      setMessages((current) => [
        ...current,
        {
          role: "user",
          content: message,
          meta: "你"
        }
      ]);

      try {
        const profile = extractProfile(structuredResult);
        const payload = {
          ...(conversationId ? { conversationId } : {}),
          ...(profile ? { profile } : {}),
          message
        };

        await callbacks.onStreamStart?.();
        let streamedReply = "";
        const streamMessage = callbacks.streamMessage ?? streamChatMessage;
        const result = await streamMessage(payload, {
          onStatus(status) {
            setStreamStatus(status);
          },
          onChunk(delta) {
            streamedReply += delta;
            setStreamReply((current) => current + delta);
            callbacks.onStreamChunk?.(delta);
          }
        });
        await callbacks.onStreamComplete?.();

        if (requestSeq.current !== seq) {
          return;
        }

        setConversationId(result.conversationId);
        safeSetLocalStorageItem(STORAGE_KEY, result.conversationId);
        const displayReply = chooseDisplayReply(result.reply, streamedReply);
        setStructuredResult(result.structuredResult ?? null);
        setCitations(Array.isArray(result.citations) ? result.citations : []);
        setMessages((current) => [
          ...current,
          {
            role: "assistant",
            content: displayReply,
            meta: "助手"
          }
        ]);
        callbacks.onReplyResolved?.(displayReply);
      } catch {
        if (requestSeq.current !== seq) {
          return;
        }

        await callbacks.onStreamError?.();
        setError("当前咨询人数较多，暂时无法完成本次查询，请稍后再试。");
        setMessages((current) => [
          ...current,
          {
            role: "assistant",
            content: "我这边暂时无法完成本次请求。请稍后重试，或换一种描述方式再问一次。",
            meta: "招生助手"
          }
        ]);
      } finally {
        if (requestSeq.current === seq) {
          setLoading(false);
          setStreamStatus("idle");
          setStreamReply("");
        }
      }
    },
    [callbacks, conversationId, structuredResult]
  );

  useEffect(() => {
    if (bootstrappedFromUrl.current) {
      return;
    }

    const searchParams = new URLSearchParams(window.location.search);
    const seededQuestion = searchParams.get("q")?.trim();
    const shouldAutoSend = searchParams.get("autosend") === "1";

    if (!seededQuestion) {
      bootstrappedFromUrl.current = true;
      return;
    }

    setInput(seededQuestion);
    bootstrappedFromUrl.current = true;

    if (shouldAutoSend) {
      void submitMessage(seededQuestion);
    }
  }, [submitMessage]);

  return {
    citations,
    conversationId,
    error,
    input,
    isStarted: requestSeq.current > 0 || messages.length > 1,
    loading,
    messages,
    setInput,
    resetSession,
    streamReply,
    streamStatus,
    structuredResult,
    submitMessage
  };
}
