"use client";

import { useCallback, useMemo, useRef, useState } from "react";

import { useTTS } from "@/components/digital-human/use-tts";

export type VoicePlaybackAvailability = "ready" | "unavailable" | "checking";

export function useVoicePlayback() {
  const tts = useTTS();
  const [availability, setAvailability] = useState<VoicePlaybackAvailability>("ready");
  const [lastError, setLastError] = useState<string | null>(null);
  const sessionPrepared = useRef(false);
  const voiceEnabled = useRef(true);
  const pendingChunks = useRef<string[]>([]);
  const finishRequested = useRef(false);

  const prepare = useCallback(async () => {
    if (!voiceEnabled.current) {
      return false;
    }

    setAvailability("checking");
    setLastError(null);

    try {
      await tts.startSession();
      sessionPrepared.current = true;
      setAvailability("ready");
      const queued = pendingChunks.current.splice(0);
      for (const chunk of queued) {
        tts.sendText(chunk);
      }
      if (finishRequested.current) {
        finishRequested.current = false;
        await tts.finishText();
        sessionPrepared.current = false;
      }
      return true;
    } catch (error) {
      sessionPrepared.current = false;
      pendingChunks.current = [];
      finishRequested.current = false;
      if (error instanceof Error && error.message === "TTS session superseded") {
        return false;
      }
      setAvailability("unavailable");
      setLastError(error instanceof Error ? error.message : "语音服务暂不可用");
      return false;
    }
  }, [tts]);

  const speakChunk = useCallback(
    (delta: string) => {
      if (!voiceEnabled.current) {
        return;
      }
      if (!sessionPrepared.current) {
        pendingChunks.current.push(delta);
        return;
      }
      tts.sendText(delta);
    },
    [tts]
  );

  const playAudioChunk = useCallback(
    (chunk: ArrayBuffer) => {
      if (!voiceEnabled.current || !sessionPrepared.current) {
        return;
      }
      tts.pushAudioData(chunk);
    },
    [tts]
  );

  const complete = useCallback(async () => {
    if (!voiceEnabled.current) {
      return;
    }
    if (!sessionPrepared.current) {
      finishRequested.current = true;
      return;
    }
    await tts.finishText();
    sessionPrepared.current = false;
  }, [tts]);

  const interrupt = useCallback(async () => {
    sessionPrepared.current = false;
    pendingChunks.current = [];
    finishRequested.current = false;
    await tts.interrupt();
  }, [tts]);

  const markUnavailable = useCallback(async () => {
    sessionPrepared.current = false;
    pendingChunks.current = [];
    finishRequested.current = false;
    setAvailability("unavailable");
    setLastError("语音服务暂不可用，但文字咨询仍可继续。");
    await tts.interrupt();
  }, [tts]);

  const clearError = useCallback(() => {
    if (availability === "unavailable") {
      setAvailability("ready");
    }
    setLastError(null);
  }, [availability]);

  const stateLabel = useMemo(() => {
    if (availability === "checking") {
      return "语音连接中";
    }
    if (availability === "unavailable") {
      return "语音暂不可用";
    }
    switch (tts.state) {
      case "connecting":
        return "语音连接中";
      case "speaking":
        return tts.isSpeaking ? "正在播报" : "待播报";
      case "error":
        return "语音暂不可用";
      default:
        return "语音已就绪";
    }
  }, [availability, tts.isSpeaking, tts.state]);

  return {
    availability,
    clearError,
    complete,
    interrupt,
    isMuted: tts.isMuted,
    isSpeaking: tts.isSpeaking,
    lastError,
    markUnavailable,
    prepare,
    playAudioChunk,
    speakChunk,
    state: tts.state,
    stateLabel,
    toggleMute: tts.toggleMute,
    usesServerVoice: tts.usesServerVoice
  };
}
