"use client";

import { cn } from "@/lib/cn";

import type { ChatMessage } from "@/components/use-chat-session";
import { FormattedMessage } from "@/components/formatted-message";

export type ChatTranscriptProps = {
  loading: boolean;
  messages: ChatMessage[];
  streamReply: string;
  streamStatus: "idle" | "resolving" | "retrieving" | "generating";
};

function getStreamStatusText(streamStatus: ChatTranscriptProps["streamStatus"]) {
  if (streamStatus === "resolving") {
    return "正在理解你的问题...";
  }

  if (streamStatus === "retrieving") {
    return "正在核对招生政策、录取数据与培养方案...";
  }

  return "正在整理回答...";
}

export function ChatTranscript({ loading, messages, streamReply, streamStatus }: ChatTranscriptProps) {
  return (
    <div className="space-y-6 px-4 py-6 sm:px-8">
      {messages.map((message, index) => {
        if (index === 0 && message.meta === "招生助手" && message.role === "assistant") {
          return null;
        }

        return (
          <div
            key={`${message.role}-${index}`}
            className={cn("flex w-full", message.role === "user" ? "justify-end" : "justify-start")}
          >
            {message.role === "assistant" ? (
              <div className="flex max-w-[92%] items-start gap-3 sm:max-w-[88%]">
                <div className="mt-1 flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-school-700 text-white shadow-sm">
                  <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 10V3L4 14h7v7l9-11h-7z" />
                  </svg>
                </div>
                <div className="rounded-[26px] rounded-tl-md border border-white/80 bg-white px-5 py-4 shadow-[0_18px_60px_rgba(15,23,42,0.06)]">
                  <p className="mb-2 text-[11px] font-semibold uppercase tracking-[0.2em] text-school-600">智能招生顾问</p>
                  <FormattedMessage className="text-[15px] leading-8 text-ink-800" text={message.content} />
                </div>
              </div>
            ) : (
              <div className="max-w-[92%] rounded-[26px] rounded-tr-md bg-ink-900 px-5 py-4 text-white shadow-[0_18px_60px_rgba(15,23,42,0.12)] sm:max-w-[82%]">
                <p className="mb-2 text-[11px] font-semibold uppercase tracking-[0.2em] text-white/60">我的问题</p>
                <FormattedMessage className="text-[15px] leading-8" text={message.content} />
              </div>
            )}
          </div>
        );
      })}

      {loading ? (
        <div className="flex w-full justify-start">
          <div className="flex max-w-[92%] items-start gap-3 sm:max-w-[88%]">
            <div className="mt-1 flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-school-700 text-white shadow-sm">
              <svg className="h-5 w-5 animate-pulse" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 10V3L4 14h7v7l9-11h-7z" />
              </svg>
            </div>
            {streamReply ? (
              <div className="rounded-[26px] rounded-tl-md border border-white/80 bg-white px-5 py-4 shadow-[0_18px_60px_rgba(15,23,42,0.06)]">
                <p className="mb-2 text-[11px] font-semibold uppercase tracking-[0.2em] text-school-600">智能招生顾问</p>
                <FormattedMessage className="text-[15px] leading-8 text-ink-800" text={streamReply} />
              </div>
            ) : (
              <div className="rounded-full border border-white/80 bg-white/80 px-4 py-2 text-xs font-semibold text-ink-500 shadow-[0_18px_60px_rgba(15,23,42,0.05)]">
                {getStreamStatusText(streamStatus)}
              </div>
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
}
