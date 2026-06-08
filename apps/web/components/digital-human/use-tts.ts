"use client";

/**
 * useTTS — React hook that wraps CosyVoice + PCMAudioPlayer.
 * Provides sendText / stop / interrupt and exposes isSpeaking state.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { CosyvoiceClient } from "./cosyvoice";
import { PCMAudioPlayer } from "./pcm-audio-player";

const WSS_BASE = "wss://dashscope.aliyuncs.com/api-ws/v1/inference";
const VOICE_ID = "cosyvoice-v3.5-plus-bailian-4a759f175e364696bc78368bce04252c";
const MODEL_NAME = "cosyvoice-v3.5-plus";
const SAMPLE_RATE = 16000;
const TTS_TRANSPORT = process.env.NEXT_PUBLIC_TTS_TRANSPORT?.trim().toLowerCase() || "dashscope";
const LOCAL_TTS_MODEL = process.env.NEXT_PUBLIC_LOCAL_TTS_MODEL?.trim() || "cosyvoice3";
const LOCAL_TTS_VOICE = process.env.NEXT_PUBLIC_LOCAL_TTS_VOICE?.trim() || "default";
const LOCAL_TTS_MAX_SEGMENT_CHARS = readPositiveInt(
  process.env.NEXT_PUBLIC_LOCAL_TTS_MAX_SEGMENT_CHARS,
  80
);
const LOCAL_TTS_SAMPLE_RATE = readPositiveInt(
  process.env.NEXT_PUBLIC_LOCAL_TTS_SAMPLE_RATE,
  24000
);
const LOCAL_TTS_MIN_SEGMENT_CHARS = readPositiveInt(
  process.env.NEXT_PUBLIC_LOCAL_TTS_MIN_SEGMENT_CHARS,
  24
);

type TTSState = "idle" | "connecting" | "speaking" | "error";
type LocalAudioItem = {
  url: string;
};

function isLocalHttpTransport() {
  return TTS_TRANSPORT === "local-http";
}

function isLocalStreamTransport() {
  return TTS_TRANSPORT === "local-stream";
}

export function isServerVoiceTransport() {
  return TTS_TRANSPORT === "server-voice";
}

function isLocalTransport() {
  return isLocalHttpTransport() || isLocalStreamTransport();
}

function getApiBaseUrl() {
  return process.env.NEXT_PUBLIC_API_BASE_URL?.trim() || "http://localhost:4000";
}

function readPositiveInt(value: string | undefined, fallback: number) {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

function cleanSpeechSegment(text: string) {
  return text
    .replace(/[`*_#>[\](){}]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function splitSpeechSegments(buffer: string, flush: boolean) {
  const segments: string[] = [];
  let rest = buffer.replace(/\r/g, "\n");

  while (true) {
    const firstTextIndex = rest.search(/\S/);
    if (firstTextIndex < 0) {
      rest = "";
      break;
    }
    if (firstTextIndex > 0) {
      rest = rest.slice(firstTextIndex);
    }

    let splitIndex = -1;
    for (let index = 0; index < rest.length; index += 1) {
      const char = rest[index] ?? "";
      if ("。！？!?；;\n".includes(char) && index >= 8) {
        const candidate = cleanSpeechSegment(rest.slice(0, index + 1));
        if (candidate.length >= LOCAL_TTS_MIN_SEGMENT_CHARS) {
          splitIndex = index + 1;
          break;
        }
      }
    }

    if (splitIndex < 0 && rest.length >= LOCAL_TTS_MAX_SEGMENT_CHARS) {
      const searchArea = rest.slice(0, LOCAL_TTS_MAX_SEGMENT_CHARS);
      const naturalBreak = Math.max(
        searchArea.lastIndexOf("，"),
        searchArea.lastIndexOf(","),
        searchArea.lastIndexOf("、"),
        searchArea.lastIndexOf("："),
        searchArea.lastIndexOf(":"),
        searchArea.lastIndexOf(" ")
      );
      splitIndex = naturalBreak >= 16 ? naturalBreak + 1 : LOCAL_TTS_MAX_SEGMENT_CHARS;
    }

    if (splitIndex < 0) {
      break;
    }

    const segment = cleanSpeechSegment(rest.slice(0, splitIndex));
    if (segment) {
      segments.push(segment);
    }
    rest = rest.slice(splitIndex);
  }

  if (flush) {
    const segment = cleanSpeechSegment(rest);
    if (segment) {
      segments.push(segment);
    }
    rest = "";
  }

  return { segments, rest };
}

async function fetchTtsToken(): Promise<string> {
  const res = await fetch(`${getApiBaseUrl()}/api/v1/tts/token`, { method: "POST" });
  const data = await res.json().catch(() => null);

  if (!res.ok) {
    const message = typeof data?.error?.message === "string" ? data.error.message : "Failed to fetch TTS token";
    throw new Error(message);
  }

  const token = data?.data?.token ?? data?.token;
  if (!token) {
    throw new Error("TTS token is missing from the response");
  }

  return token;
}

async function fetchLocalSpeech(
  input: string,
  signal?: AbortSignal
): Promise<Blob> {
  const requestInit: RequestInit = {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      input,
      model: LOCAL_TTS_MODEL,
      voice: LOCAL_TTS_VOICE,
    }),
  };
  if (signal) {
    requestInit.signal = signal;
  }

  const res = await fetch(`${getApiBaseUrl()}/api/v1/tts/speech`, requestInit);

  if (!res.ok) {
    const data = await res.json().catch(() => null);
    const message =
      typeof data?.error?.message === "string"
        ? data.error.message
        : "Failed to synthesize speech";
    throw new Error(message);
  }

  return res.blob();
}

async function streamLocalSpeech(
  input: string,
  onAudioData: (chunk: ArrayBuffer) => void,
  signal?: AbortSignal
): Promise<void> {
  const requestInit: RequestInit = {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      input,
      model: LOCAL_TTS_MODEL,
      voice: LOCAL_TTS_VOICE,
    }),
  };
  if (signal) {
    requestInit.signal = signal;
  }

  const res = await fetch(`${getApiBaseUrl()}/api/v1/tts/stream`, requestInit);

  if (!res.ok) {
    const data = await res.json().catch(() => null);
    const message =
      typeof data?.error?.message === "string"
        ? data.error.message
        : "Failed to stream speech";
    throw new Error(message);
  }

  const reader = res.body?.getReader();
  if (!reader) {
    throw new Error("Streaming TTS response body is unavailable");
  }

  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) {
        break;
      }
      if (value && value.byteLength > 0) {
        onAudioData(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength));
      }
    }
  } finally {
    reader.releaseLock();
  }
}

export function useTTS() {
  const [state, setState] = useState<TTSState>("idle");
  const [isSpeaking, setIsSpeaking] = useState(false);
  const [isMuted, setIsMuted] = useState(false);

  const clientRef = useRef<CosyvoiceClient | null>(null);
  const playerRef = useRef<PCMAudioPlayer | null>(null);
  const sessionActive = useRef(false);
  const sessionRunId = useRef(0);
  const localTextBuffer = useRef("");
  const localSynthesisQueue = useRef<string[]>([]);
  const localReadyAudioQueue = useRef<LocalAudioItem[]>([]);
  const localSynthesisPromise = useRef<Promise<void> | null>(null);
  const localPlaybackPromise = useRef<Promise<void> | null>(null);
  const localFinishRequested = useRef(false);
  const localError = useRef<Error | null>(null);
  const localAudioRef = useRef<HTMLAudioElement | null>(null);
  const localAudioUrlRef = useRef<string | null>(null);
  const localSpeechAbortRef = useRef<AbortController | null>(null);
  const isMutedRef = useRef(isMuted);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      clientRef.current?.close();
      playerRef.current?.stop();
      localSpeechAbortRef.current?.abort();
      localAudioRef.current?.pause();
      if (localAudioUrlRef.current) {
        URL.revokeObjectURL(localAudioUrlRef.current);
      }
    };
  }, []);

  useEffect(() => {
    isMutedRef.current = isMuted;
  }, [isMuted]);

  const cleanupLocalAudio = useCallback(() => {
    localAudioRef.current?.pause();
    localAudioRef.current = null;
    if (localAudioUrlRef.current) {
      URL.revokeObjectURL(localAudioUrlRef.current);
      localAudioUrlRef.current = null;
    }
    for (const item of localReadyAudioQueue.current.splice(0)) {
      URL.revokeObjectURL(item.url);
    }
  }, []);

  const settleLocalIfFinished = useCallback(() => {
    if (isLocalStreamTransport()) {
      return;
    }
    if (
      localFinishRequested.current &&
      localSynthesisQueue.current.length === 0 &&
      localReadyAudioQueue.current.length === 0 &&
      !localSynthesisPromise.current &&
      !localPlaybackPromise.current
    ) {
      setIsSpeaking(false);
      setState("idle");
      sessionActive.current = false;
    }
  }, []);

  const processLocalPlaybackQueue = useCallback(() => {
    if (localPlaybackPromise.current) {
      return localPlaybackPromise.current;
    }

    const runId = sessionRunId.current;
    const promise = (async () => {
      while (sessionRunId.current === runId) {
        const item = localReadyAudioQueue.current.shift();
        if (!item) {
          break;
        }

        if (isMutedRef.current) {
          URL.revokeObjectURL(item.url);
          continue;
        }

        localAudioUrlRef.current = item.url;
        const audio = new Audio(item.url);
        localAudioRef.current = audio;

        try {
          await new Promise<void>((resolve, reject) => {
            audio.onplay = () => {
              setState("speaking");
              setIsSpeaking(true);
            };
            audio.onended = () => {
              resolve();
            };
            audio.onerror = () => {
              reject(new Error("Local TTS audio playback failed"));
            };
            audio.play().catch(reject);
          });
        } catch (error) {
          if (sessionRunId.current === runId) {
            localError.current =
              error instanceof Error ? error : new Error("Local TTS playback failed");
            setIsSpeaking(false);
            setState("error");
          }
          return;
        } finally {
          if (localAudioRef.current === audio) {
            localAudioRef.current.pause();
            localAudioRef.current = null;
          }
          if (localAudioUrlRef.current === item.url) {
            localAudioUrlRef.current = null;
          }
          URL.revokeObjectURL(item.url);
        }
      }
    })().finally(() => {
      if (localPlaybackPromise.current === promise) {
        localPlaybackPromise.current = null;
      }
      if (
        sessionRunId.current === runId &&
        localReadyAudioQueue.current.length > 0 &&
        !localError.current
      ) {
        void processLocalPlaybackQueue();
        return;
      }
      settleLocalIfFinished();
    });

    localPlaybackPromise.current = promise;
    return promise;
  }, [settleLocalIfFinished]);

  const processLocalSynthesisQueue = useCallback(() => {
    if (localSynthesisPromise.current) {
      return localSynthesisPromise.current;
    }

    const runId = sessionRunId.current;
    const promise = (async () => {
      while (sessionRunId.current === runId) {
        const segment = localSynthesisQueue.current.shift();
        if (!segment) {
          break;
        }

        if (isMutedRef.current) {
          continue;
        }

        const controller = new AbortController();
        localSpeechAbortRef.current = controller;
        if (!localPlaybackPromise.current) {
          setState("connecting");
        }

        try {
          if (isLocalStreamTransport()) {
            await streamLocalSpeech(
              segment,
              (chunk) => {
                if (
                  sessionRunId.current === runId &&
                  !isMutedRef.current &&
                  playerRef.current?.ready
                ) {
                  playerRef.current.pushPCM(chunk);
                  setIsSpeaking(true);
                  setState("speaking");
                }
              },
              controller.signal
            );
            continue;
          }

          const audioBlob = await fetchLocalSpeech(segment, controller.signal);
          if (sessionRunId.current !== runId) {
            return;
          }

          localReadyAudioQueue.current.push({ url: URL.createObjectURL(audioBlob) });
          void processLocalPlaybackQueue();
        } catch (error) {
          if (!controller.signal.aborted && sessionRunId.current === runId) {
            const normalizedError =
              error instanceof Error ? error : new Error("Local TTS synthesis failed");
            localError.current = normalizedError;
            setIsSpeaking(false);
            setState("error");
          }
          return;
        } finally {
          if (localSpeechAbortRef.current === controller) {
            localSpeechAbortRef.current = null;
          }
        }
      }
    })().finally(() => {
      if (localSynthesisPromise.current === promise) {
        localSynthesisPromise.current = null;
      }
      if (
        sessionRunId.current === runId &&
        localSynthesisQueue.current.length > 0 &&
        !localError.current
      ) {
        void processLocalSynthesisQueue();
        return;
      }
      settleLocalIfFinished();
    });

    localSynthesisPromise.current = promise;
    return promise;
  }, [processLocalPlaybackQueue, settleLocalIfFinished]);

  const enqueueLocalSpeech = useCallback(
    (text: string, flush = false) => {
      localTextBuffer.current += text;
      const { segments, rest } = splitSpeechSegments(localTextBuffer.current, flush);
      localTextBuffer.current = rest;
      if (segments.length > 0) {
        localSynthesisQueue.current.push(...segments);
        void processLocalSynthesisQueue();
      }
    },
    [processLocalSynthesisQueue]
  );

  /**
   * Start a new TTS session — call this when AI starts streaming response.
   * Returns quickly after the WebSocket handshake completes.
   */
  const startSession = useCallback(async () => {
    const runId = sessionRunId.current + 1;
    sessionRunId.current = runId;

    if (clientRef.current) {
      clientRef.current.close();
      clientRef.current = null;
    }
    if (playerRef.current) {
      await playerRef.current.stop();
      playerRef.current = null;
    }
    localSpeechAbortRef.current?.abort();
    localSpeechAbortRef.current = null;
    cleanupLocalAudio();
    localTextBuffer.current = "";
    localSynthesisQueue.current = [];
    localReadyAudioQueue.current = [];
    localFinishRequested.current = false;
    localError.current = null;

    setState("connecting");
    sessionActive.current = true;

    if (isLocalHttpTransport()) {
      setIsSpeaking(false);
      setState("speaking");
      return;
    }

    if (isLocalStreamTransport() || isServerVoiceTransport()) {
      const player = new PCMAudioPlayer(LOCAL_TTS_SAMPLE_RATE, {
        onPlaybackComplete: () => {
          setIsSpeaking(false);
          setState("idle");
          sessionActive.current = false;
        },
        onSpeakingChange: (speaking) => {
          setIsSpeaking(speaking);
        }
      });

      await player.connect();
      if (sessionRunId.current !== runId) {
        await player.stop();
        throw new Error("TTS session superseded");
      }
      playerRef.current = player;
      setIsSpeaking(false);
      setState("speaking");
      return;
    }

    try {
      const token = await fetchTtsToken();
      if (sessionRunId.current !== runId) {
        throw new Error("TTS session superseded");
      }
      const wssUrl = `${WSS_BASE}/?api_key=${token}`;

      const player = new PCMAudioPlayer(SAMPLE_RATE, {
        onPlaybackComplete: () => {
          setIsSpeaking(false);
          setState("idle");
          sessionActive.current = false;
        },
        onSpeakingChange: (speaking) => {
          setIsSpeaking(speaking);
        }
      });

      await player.connect();
      if (sessionRunId.current !== runId) {
        await player.stop();
        throw new Error("TTS session superseded");
      }
      playerRef.current = player;

      const client = new CosyvoiceClient(wssUrl, VOICE_ID, MODEL_NAME);
      await client.connect({
        onAudioData: (data) => {
          if (!isMuted) {
            player.pushPCM(data);
          }
        },
        onTaskFinished: () => {
          player.sendTtsFinished();
        }
      });

      if (sessionRunId.current !== runId) {
        client.close();
        await player.stop();
        throw new Error("TTS session superseded");
      }
      clientRef.current = client;
      setState("speaking");
    } catch (err) {
      if (sessionRunId.current === runId) {
        setState("error");
        sessionActive.current = false;
      }
      throw err instanceof Error ? err : new Error("TTS session failed to start");
    }
  }, [cleanupLocalAudio, isMuted]);

  /** Send a text chunk to TTS (call per SSE delta) */
  const sendText = useCallback((text: string) => {
    if (isServerVoiceTransport()) {
      return;
    }

    if (isLocalTransport()) {
      enqueueLocalSpeech(text);
      return;
    }

    if (clientRef.current?.connected) {
      clientRef.current.sendText(text);
      setIsSpeaking(true);
    }
  }, [enqueueLocalSpeech]);

  const pushAudioData = useCallback((chunk: ArrayBuffer) => {
    if (!isServerVoiceTransport() || isMutedRef.current || !playerRef.current?.ready) {
      return;
    }
    playerRef.current.pushPCM(chunk);
    setIsSpeaking(true);
    setState("speaking");
  }, []);

  /** Signal end of text input */
  const finishText = useCallback(async () => {
    if (isServerVoiceTransport()) {
      playerRef.current?.sendTtsFinished();
      return;
    }

    if (isLocalTransport()) {
      localFinishRequested.current = true;
      enqueueLocalSpeech("", true);
      await localSynthesisPromise.current;
      if (isLocalStreamTransport()) {
        playerRef.current?.sendTtsFinished();
      } else {
        await localPlaybackPromise.current;
      }

      if (localError.current) {
        const error = localError.current;
        localError.current = null;
        throw error;
      }

      if (
        localSynthesisQueue.current.length === 0 &&
        localReadyAudioQueue.current.length === 0 &&
        !isLocalStreamTransport()
      ) {
        setIsSpeaking(false);
        setState("idle");
        sessionActive.current = false;
      }
      return;
    }

    if (clientRef.current?.connected) {
      await clientRef.current.stop();
    }
  }, [enqueueLocalSpeech]);

  /** Hard interrupt — stop everything immediately */
  const interrupt = useCallback(async () => {
    sessionRunId.current += 1;
    sessionActive.current = false;
    clientRef.current?.close();
    clientRef.current = null;
    if (playerRef.current) {
      await playerRef.current.stop();
      playerRef.current = null;
    }
    localSpeechAbortRef.current?.abort();
    localSpeechAbortRef.current = null;
    localTextBuffer.current = "";
    localSynthesisQueue.current = [];
    localReadyAudioQueue.current = [];
    localFinishRequested.current = false;
    localError.current = null;
    cleanupLocalAudio();
    setIsSpeaking(false);
    setState("idle");
  }, [cleanupLocalAudio]);

  const toggleMute = useCallback(() => {
    setIsMuted((prev) => !prev);
  }, []);

  return {
    state,
    isSpeaking,
    isMuted,
    startSession,
    sendText,
    pushAudioData,
    usesServerVoice: isServerVoiceTransport(),
    finishText,
    interrupt,
    toggleMute,
  };
}
