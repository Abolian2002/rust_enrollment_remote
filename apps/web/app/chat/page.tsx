"use client";

import type { FormEvent, ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useChatSession } from "@/components/use-chat-session";
import { useVoicePlayback } from "@/components/digital-human/use-voice-playback";
import { FormattedMessage } from "@/components/formatted-message";
import { createPublicTicket, stopActiveVoiceStream, streamVoiceChatMessage } from "@/lib/api-client";
import { cn } from "@/lib/cn";

const hotCategories = [
  { label: "院校与校园介绍", icon: ClipboardIcon },
  { label: "历年分数与位次", icon: TrendIcon },
  { label: "招生计划与专业", icon: FileIcon },
  { label: "报考录取政策", icon: ScaleIcon },
  { label: "专项与公费师范", icon: RibbonIcon },
  { label: "就读与就业发展", icon: BuildingIcon }
] as const;

const defaultHotCategory = hotCategories[0].label;

function getStreamStatusText(streamStatus: "idle" | "resolving" | "retrieving" | "generating") {
  if (streamStatus === "resolving") {
    return "正在理解你的问题...";
  }

  if (streamStatus === "retrieving") {
    return "正在核对招生政策、录取数据与培养方案...";
  }

  return "正在整理回答...";
}

const hotQuestionGroups: Record<string, string[][]> = {
  "院校与校园介绍": [
    [
      "哈尔滨师范大学有哪些优势专业？",
      "学校有几个校区？分别在什么地方？",
      "学校的住宿条件怎么样？",
      "学校食堂的饭菜种类和价格如何？",
      "学校有哪些特色社团活动？",
      "师范专业的师资力量如何？"
    ],
    [
      "哈尔滨师范大学是一所什么类型的大学？",
      "松北校区和江南校区有什么区别？",
      "学校图书馆和自习环境怎么样？",
      "新生入学后主要在哪个校区学习？",
      "学校周边交通和生活便利吗？",
      "学校有哪些值得了解的校园传统？"
    ]
  ],
  "历年分数与位次": [
    [
      "历年录取分数和位次怎么查？",
      "某专业去年录取位次是多少？",
      "美术类专业录取分数怎么查？",
      "艺术类专业文化课要求怎么看？",
      "各专业录取分数区间怎么查？",
      "分数相同的考生，会优先录取谁？"
    ]
  ],
  "招生计划与专业": [
    [
      "2025年学校面向哪些省份招生？",
      "各专业招生计划人数在哪里查？",
      "学校有哪些师范类专业？",
      "学校有哪些新增专业？",
      "行知实验班是什么？有哪些专业？",
      "某专业培养目标、课程和学分怎么查？"
    ]
  ],
  "报考录取政策": [
    [
      "学校专业录取规则是什么？",
      "专业志愿之间有级差吗？",
      "服从调剂还会被退档吗？",
      "贵校哪些专业对单科成绩有要求？",
      "哪些专业有外语语种限制？",
      "各招生专业选考科目在哪里查？"
    ]
  ],
  "专项与公费师范": [
    [
      "学校有没有专项计划招生？",
      "公费师范生政策按什么执行？",
      "公费师范生有哪些招生专业？",
      "专项计划的录取分数有固定优惠吗？",
      "贵校是否开设少数民族预科班？",
      "学校有单招计划吗？"
    ]
  ],
  "就读与就业发展": [
    [
      "学校转专业有什么基本规则？",
      "师范生毕业后一定能当老师吗？",
      "学校升学和保研情况怎么样？",
      "各专业对学分有什么要求？",
      "学校有助学金或助学贷款吗？怎么办理？",
      "你们学校有哪些专业招研究生？"
    ]
  ]
};

const quickLinks = [
  {
    label: "招生计划",
    href: "https://v.360eol.com/web/admission/admmisonPlanIndexForPc.do?appPrefix=hrbnu",
    icon: ClipboardIcon,
    featured: true
  },
  {
    label: "录取分数",
    href: "https://v.360eol.com/web/admission/splineForPc.do?appPrefix=hrbnu",
    icon: TrendIcon,
    featured: true
  },
  { label: "官网", href: "http://www.hrbnu.edu.cn/index.htm", icon: GlobeIcon },
  { label: "简介", href: "http://www.hrbnu.edu.cn/xxgk/xxjj.htm", icon: BookIcon },
  { label: "科研", href: "http://www.hrbnu.edu.cn/kxyj1/kyjg.htm", icon: MicroscopeIcon },
  { label: "要闻", href: "http://www.hrbnu.edu.cn/index/sdyw.htm", icon: NewsIcon }
];

type RichMediaAnswer = {
  question: string;
  answer: string;
  images?: Array<{ src: string; alt: string }>;
  video?: { src: string; title: string };
};

type LocalRichMessage =
  | {
      role: "user";
      content: string;
    }
  | {
      role: "assistant";
      content: string;
      images?: Array<{ src: string; alt: string }>;
      video?: { src: string; title: string };
    };

const campusGallery = [
  { src: "/coze-replica/rich/campus-1.jpg", alt: "哈尔滨师范大学校园风景一" },
  { src: "/coze-replica/rich/campus-2.jpg", alt: "哈尔滨师范大学校园风景二" },
  { src: "/coze-replica/rich/campus-3.jpg", alt: "哈尔滨师范大学校园风景三" },
  { src: "/coze-replica/rich/campus-4.jpg", alt: "哈尔滨师范大学校园风景四" },
  { src: "/coze-replica/rich/campus-5.jpg", alt: "哈尔滨师范大学校园风景五" }
];

const localRichAnswers: Record<string, RichMediaAnswer> = {
  "哈尔滨师范大学有哪些优势专业？": {
    question: "哈尔滨师范大学有哪些优势专业？",
    answer:
      "哈喽～哈师大的优势专业可不少，主要集中在师范类和特色学科领域，给你梳理一下👇\n\n✅ 国家级一流本科专业建设点：教育学、汉语言文学、英语、数学与应用数学、物理学、化学、生物科学、地理科学、思想政治教育、历史学、音乐学、美术学、体育教育、俄语、心理学、计算机科学与技术等，都是学校比较有代表性的优势方向。\n\n✅ 省级重点/特色专业：俄语、新闻学、旅游管理、舞蹈编导等也很有辨识度，像书法学这类专业在省内也有较高知名度。\n\n✅ 师范类王牌方向：作为省属师范强校，教育学、汉语言文学、英语、数学与应用数学等专业师资扎实，在省内基础教育系统认可度比较高。\n\n另外，学校的艺术学科也很有特色，音乐、美术、舞蹈等方向经常参与各类展演和比赛。如果你有具体专业方向，我可以继续帮你细讲招生情况 and 就业去向。",
    video: { src: "/coze-replica/rich/advantages.mp4", title: "哈尔滨师范大学招生宣传片" }
  },
  "学校有几个校区？分别在什么地方？": {
    question: "学校有几个校区？分别在什么地方？",
    answer:
      "哈喽～哈师大目前主要有两个校区，位置和特点各有不同👇\n\n📍 松北校区（主校区）：地址在哈尔滨市呼兰区利民经济开发区师大路1号。这里是学校规模较大、设施较新的校区，大部分学院都在这里，教学楼、图书馆、体育馆、宿舍和食堂配套比较完整。\n\n📍 江南校区：地址在哈尔滨市南岗区和兴路50号。这个校区位于市区，交通便利，周边生活配套成熟，部分文科、艺术类相关学院在这里，校园更有老校区的历史感和文化氛围。\n\n两个校区之间日常往来相对方便，具体专业在哪个校区，建议以当年录取通知和学院安排为准。",
    images: campusGallery
  },
  "学校的住宿条件怎么样？": {
    question: "学校的住宿条件怎么样？",
    answer:
      "哈哈，这确实是新生很关心的问题。哈师大住宿会按校区、学院和专业统一安排，整体能满足日常学习生活需要👇\n\n✨ 松北校区：本科生宿舍以多人间为主，常见配置包括床位、书桌、衣柜等，部分楼栋条件更新一些。楼内一般配有公共洗漱、洗衣、热水等生活设施，日常学习和生活比较方便。\n\n🍂 江南校区：宿舍类型会因楼栋不同有所差异，部分宿舍为四人间或六人间，整体生活配套成熟，周边交通和餐饮也比较便利。\n\n💡 小提示：宿舍分配通常按学院、专业和当年住宿资源统筹安排，具体房型、楼栋和设施以入学报到时学校公布为准。可以提前准备收纳用品，让自己的小空间更舒服。",
    images: campusGallery.slice(0, 4)
  },
  "学校食堂的饭菜种类和价格如何？": {
    question: "学校食堂的饭菜种类和价格如何？",
    answer:
      "作为干饭人，这个问题必须认真回答😋 哈师大两个校区都有食堂和餐饮窗口，整体选择比较丰富。\n\n✅ 种类方面：常见的东北家常菜、盖饭、面食、米线、麻辣烫、炒饭、小吃、早餐粥点等基本都有，也会有一些地方风味窗口，能照顾不同口味。\n\n✅ 价格方面：学生食堂整体偏亲民，普通一餐通常十几元左右可以解决，具体会根据菜品、窗口和饭量不同有所差异。\n\n✅ 体验方面：高峰期人会比较多，建议错峰就餐。不同食堂和窗口口味差异也比较明显，入学后可以慢慢探索自己的固定菜单。",
    images: campusGallery.slice(0, 4)
  },
  "学校有哪些特色社团活动？": {
    question: "学校有哪些特色社团活动？",
    answer:
      "哈师大的校园活动和社团类型比较丰富，适合不同兴趣的同学参与👇\n\n✅ 文艺类：合唱、舞蹈、话剧、民乐、吉他、书法、汉服、动漫等社团都比较受欢迎，迎新晚会、校园文化节、草地音乐类活动也很有参与感。\n\n✅ 学术实践类：辩论、模拟联合国、支教、志愿服务、创新创业、教师技能训练等活动，比较适合想提升表达、组织和实践能力的同学。\n\n✅ 体育兴趣类：篮球、羽毛球、排球、跑步、街舞等运动社团和校内比赛也比较常见。\n\n每年开学季一般会有社团招新，建议你到时候多逛逛，先体验再决定加入哪个组织。",
    images: campusGallery.slice(0, 4)
  },
  "师范专业的师资力量如何？": {
    question: "师范专业的师资力量如何？",
    answer:
      "作为哈师大的优势特色，师范专业的师资和培养体系是学校很重要的底盘👍\n\n首先，学校不少师范类专业依托教育学、汉语言文学、英语、数学与应用数学、物理学、化学、生物科学、地理科学、思想政治教育、历史学、音乐学、美术学、体育教育、俄语、心理学、计算机科学与技术等优势学科建设，课程体系比较扎实。\n\n其次，师范专业很重视教学能力培养，除了专业基础课，也会涉及教育学、心理学、课程教学论、教师技能训练、教育实习等内容，帮助学生从“会学”逐步走向“会教”。\n\n另外，很多老师长期从事教师教育和基础教育研究，会结合真实教学案例讲授课堂设计、班级管理和学生沟通等内容。整体来看，如果目标是从教，哈师大的师范培养基础还是比较有优势的。"
  }
};

function wait(ms: number) {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

function chunkTextForTts(text: string): string[] {
  const normalized = text
    .replace(/[✅📍✨🍂💡😋👍👇～]/gu, "")
    .replace(/\n{2,}/gu, "\n")
    .trim();

  const roughChunks = normalized
    .split(/(?<=[。！？；;.!?])|\n/gu)
    .map((item) => item.trim())
    .filter(Boolean);

  const chunks: string[] = [];
  for (const item of roughChunks) {
    if (item.length <= 90) {
      chunks.push(item);
      continue;
    }

    for (let index = 0; index < item.length; index += 80) {
      chunks.push(item.slice(index, index + 80));
    }
  }

  return chunks;
}

export default function ChatPage() {
  // Redirect to HTTPS in production to ensure a secure context for AudioWorklet support
  useEffect(() => {
    if (
      typeof window !== "undefined" &&
      window.location.protocol === "http:" &&
      window.location.hostname !== "localhost" &&
      window.location.hostname !== "127.0.0.1"
    ) {
      window.location.replace(
        `https://${window.location.hostname}${window.location.pathname}${window.location.search}`
      );
    }
  }, []);

  const messagesEndRef = useRef<HTMLDivElement>(null);
  const [localRichMessages, setLocalRichMessages] = useState<LocalRichMessage[]>([]);
  const [isDrawerOpen, setIsDrawerOpen] = useState(false);
  const [isDhModalOpen, setIsDhModalOpen] = useState(false);
  const [isTicketModalOpen, setIsTicketModalOpen] = useState(false);
  const [selectedModel, setSelectedModel] = useState<string>("ours");

  const voice = useVoicePlayback();
  const {
    error,
    input,
    loading,
    messages,
    resetSession,
    setInput,
    streamReply,
    streamStatus,
    submitMessage
  } = useChatSession({
    model: selectedModel,
    onStreamStart: async () => {
      if (voice.usesServerVoice) {
        stopActiveVoiceStream();
        void voice.prepare().then((ready) => {
          if (!ready) {
            void voice.markUnavailable();
          }
        }).catch(() => {
          void voice.markUnavailable();
        });
        return;
      }
      try {
        await voice.prepare();
      } catch {
        await voice.markUnavailable();
      }
    },
    onStreamChunk: (delta) => {
      voice.speakChunk(delta);
    },
    ...(voice.usesServerVoice
      ? {
          streamMessage: (payload, handlers) =>
            streamVoiceChatMessage(payload, {
              ...handlers,
              onAudioChunk: (chunk) => {
                voice.playAudioChunk(chunk);
              },
              onAudioDone: () => {
                void voice.complete();
              }
            })
        }
      : {}),
    onStreamComplete: () => {
      if (voice.usesServerVoice) {
        return;
      }
      void voice.complete();
    },
    onStreamError: () => {
      void voice.interrupt();
    }
  });

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [localRichMessages, messages, streamReply]);

  const submit = useCallback(
    async (message: string) => {
      setLocalRichMessages([]);
      if (voice.usesServerVoice) {
        stopActiveVoiceStream();
      }
      // Interrupt previous playback and start preparing voice context synchronously in the user click gesture tick
      void voice.interrupt();
      void voice.prepare();
      await submitMessage(message);
    },
    [submitMessage, voice]
  );

  const askHotQuestion = useCallback(
    async (message: string) => {
      const localAnswer = localRichAnswers[message];
      if (!localAnswer) {
        await submit(message);
        return;
      }

      setInput("");
      setLocalRichMessages((current) => [
        ...current,
        { role: "user", content: localAnswer.question },
        {
          role: "assistant",
          content: localAnswer.answer,
          ...(localAnswer.images ? { images: localAnswer.images } : {}),
          ...(localAnswer.video ? { video: localAnswer.video } : {})
        }
      ]);
      if (voice.usesServerVoice) {
        // For server voice, speakText handles its own preparation/play
        void (async () => {
          try {
            await voice.speakText(localAnswer.answer);
          } catch {
            await voice.markUnavailable();
          }
        })();
        return;
      }

      // Interrupt previous playback and start preparing voice context synchronously in the user click gesture tick
      void voice.interrupt();
      const prepPromise = voice.prepare();
      void (async () => {
        try {
          const ready = await prepPromise;
          if (!ready) {
            return;
          }
          for (const chunk of chunkTextForTts(localAnswer.answer)) {
            voice.speakChunk(`${chunk} `);
            await wait(120);
          }

          await voice.complete();
        } catch {
          await voice.markUnavailable();
        }
      })();
    },
    [setInput, submit, voice]
  );

  const clearChat = useCallback(() => {
    setLocalRichMessages([]);
    resetSession();
    void voice.interrupt();
  }, [resetSession, voice]);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    await submit(input);
  }

  const visibleMessages = messages.filter((message, index) => !(index === 0 && message.role === "assistant"));
  const latestAssistant = [...messages].reverse().find((message) => message.role === "assistant")?.content;

  return (
    <main className="h-[100svh] overflow-hidden bg-school-50 text-slate-800">
      <div className="flex h-full flex-col relative bg-school-radial bg-academy-texture">
        {/* Glow decoration blobs */}
        <div className="absolute top-[-10%] left-[-10%] w-[50%] h-[50%] bg-[#ce3459]/5 rounded-full blur-[120px] pointer-events-none z-0" />
        <div className="absolute bottom-[-10%] right-[-10%] w-[50%] h-[50%] bg-gold-400/5 rounded-full blur-[120px] pointer-events-none z-0" />

        <Header
          onOpenDrawer={() => setIsDrawerOpen(true)}
          selectedModel={selectedModel}
          onModelChange={setSelectedModel}
        />

        {/* 3-Column Layout Container */}
        <section className="relative z-10 min-h-0 flex-1 overflow-hidden">
          <div className="mx-auto flex w-full max-w-[1600px] gap-6 p-4 md:p-6 h-full">
            
            {/* Left Column: Digital Human Advisor (Desktop) */}
            <aside className="hidden lg:flex w-[320px] xl:w-[360px] shrink-0 flex-col gap-5 h-full">
              {/* Digital Human Stage */}
              <div className="flex-1 min-h-0 flex flex-col rounded-3xl overflow-hidden glass-panel shadow-soft border border-white/50">
                <DigitalHumanStage
                  isSpeaking={voice.isSpeaking}
                  stateLabel={voice.stateLabel}
                  isMuted={voice.isMuted}
                  onMuteToggle={voice.toggleMute}
                  onInterrupt={voice.interrupt}
                  availability={voice.availability}
                />
              </div>

              {/* Quick Links Card */}
              <div className="rounded-3xl p-5 glass-panel shadow-soft border border-white/50 shrink-0">
                <h3 className="text-xs font-extrabold text-school-900 mb-3.5 flex items-center gap-2">
                  <span className="h-4 w-1 rounded bg-[#ce3459]" />
                  快捷服务通道
                </h3>
                <QuickAccess />
              </div>
            </aside>

            {/* Middle Column: Chat Workspace */}
            <section className="flex flex-1 flex-col rounded-3xl glass-panel shadow-soft border border-white/50 overflow-hidden h-full">
              <div className="flex-1 min-h-0 flex flex-col relative">
                
                {/* Scrollable Conversation Stream */}
                <div className="flex-1 overflow-y-auto px-4 py-6 sm:px-6 md:px-8 custom-scrollbar">
                  <div className="mx-auto w-full max-w-[800px] flex flex-col gap-6">
                    {/* Welcome Bubble */}
                    <WelcomeBubble
                      isSpeaking={voice.isSpeaking}
                      latestAssistant={latestAssistant ?? null}
                      onReplay={() => {
                        if (latestAssistant) {
                          if (voice.usesServerVoice) {
                            void voice.speakText(latestAssistant);
                          } else {
                            voice.speakChunk(latestAssistant);
                            void voice.complete();
                          }
                        }
                      }}
                    />

                    {/* Standard messages */}
                    {visibleMessages.map((message, index) => (
                      <ChatBubble key={`${message.role}-${index}-${message.content.slice(0, 12)}`} role={message.role}>
                        {message.content}
                      </ChatBubble>
                    ))}

                    {/* Local Rich Answer messages */}
                    {localRichMessages.map((message, index) =>
                      message.role === "user" ? (
                        <ChatBubble key={`local-user-${index}`} role="user">
                          {message.content}
                        </ChatBubble>
                      ) : (
                        <RichAnswerBubble key={`local-assistant-${index}`} message={message} />
                      )
                    )}

                    {/* Loading / Streaming State */}
                    {loading && streamReply ? (
                      <ChatBubble role="assistant">
                        {streamReply}
                      </ChatBubble>
                    ) : loading ? (
                      <AssistantStatusLine text={getStreamStatusText(streamStatus)} />
                    ) : null}

                    {/* Error State */}
                    {error ? (
                      <div className="rounded-2xl border border-rose-100 bg-rose-50 px-4 py-3.5 text-sm text-rose-700 shadow-sm">
                        {error}
                      </div>
                    ) : null}

                    <div ref={messagesEndRef} />
                  </div>
                </div>

                {/* Input Area Docked at Bottom */}
                <InputDock
                  disabled={loading}
                  input={input}
                  onAsk={submit}
                  onInputChange={setInput}
                  onInterrupt={() => {
                    void voice.interrupt();
                  }}
                  onClear={clearChat}
                  onSubmit={handleSubmit}
                  onOpenDrawer={() => setIsDrawerOpen(true)}
                  onOpenTicket={() => setIsTicketModalOpen(true)}
                />
              </div>
            </section>

            {/* Right Column: Hot Questions (Desktop XL only) */}
            <aside className="hidden xl:flex w-[320px] shrink-0 flex-col rounded-3xl glass-panel shadow-soft border border-white/50 overflow-hidden h-full p-5">
              <HotQuestions onAsk={askHotQuestion} />
            </aside>

          </div>
        </section>

        {/* Floating Digital Human PIP Bubble on Mobile/Tablet */}
        <button
          type="button"
          onClick={() => setIsDhModalOpen(true)}
          className={cn(
            "fixed bottom-24 right-5 z-40 flex h-16 w-16 items-center justify-center rounded-full border-2 border-[#ce3459] bg-slate-950 shadow-xl lg:hidden overflow-hidden hover-zoom active:scale-95 transition-all duration-300",
            voice.isSpeaking && "ring-4 ring-[#ce3459]/30"
          )}
          title="打开数字人顾问"
        >
          <img 
            src="/coze-replica/muyang-listen.gif" 
            alt="沐阳头像" 
            className="h-full w-full object-cover object-top" 
          />
          {/* Wave effect overlay when speaking */}
          {voice.isSpeaking && (
            <div className="absolute inset-0 flex items-center justify-center bg-slate-950/40 backdrop-blur-[1px]">
              <span className="flex items-end gap-0.5 h-6">
                <span className="w-0.5 bg-gold-400 rounded animate-pulse h-3" />
                <span className="w-0.5 bg-gold-400 rounded animate-pulse h-5" style={{ animationDelay: "0.15s" }} />
                <span className="w-0.5 bg-gold-400 rounded animate-pulse h-4" style={{ animationDelay: "0.3s" }} />
              </span>
            </div>
          )}
        </button>

        {/* Slide-over Drawer for HotQuestions on mobile/tablet */}
        {isDrawerOpen && (
          <div className="fixed inset-0 z-50 flex justify-end xl:hidden">
            {/* Backdrop */}
            <div className="absolute inset-0 bg-slate-950/60 backdrop-blur-sm transition-opacity" onClick={() => setIsDrawerOpen(false)} />
            {/* Drawer Body */}
            <div className="relative w-full max-w-[360px] h-full bg-white p-6 shadow-2xl flex flex-col z-10 border-l border-slate-200 animate-slide-in">
              <button 
                type="button" 
                onClick={() => setIsDrawerOpen(false)} 
                className="absolute top-4 right-4 text-slate-400 hover:text-slate-700 p-2 rounded-full hover:bg-slate-100 transition"
              >
                <CloseIcon className="h-5 w-5" />
              </button>
              <div className="flex-1 overflow-y-auto pr-1 mt-4 custom-scrollbar">
                <HotQuestions onAsk={(q) => {
                  setIsDrawerOpen(false);
                  void askHotQuestion(q);
                }} />
              </div>
            </div>
          </div>
        )}

        {/* Digital Human Modal for Mobile/Tablet */}
        {isDhModalOpen && (
          <div className="fixed inset-0 z-50 flex items-center justify-center p-4 lg:hidden">
            {/* Backdrop */}
            <div className="absolute inset-0 bg-slate-950/80 backdrop-blur-md" onClick={() => setIsDhModalOpen(false)} />
            {/* Modal Body */}
            <div className="relative w-full max-w-[400px] aspect-[3/4] bg-school-900 rounded-3xl overflow-hidden shadow-2xl flex flex-col z-10 border border-white/10">
              <button
                type="button"
                onClick={() => setIsDhModalOpen(false)}
                className="absolute top-4 right-4 z-20 bg-slate-950/60 hover:bg-slate-950/80 text-white rounded-full p-2 backdrop-blur-md border border-white/10 transition"
              >
                <CloseIcon className="h-5 w-5" />
              </button>
              <div className="flex-1 min-h-0 flex flex-col">
                <DigitalHumanStage
                  isSpeaking={voice.isSpeaking}
                  stateLabel={voice.stateLabel}
                  isMuted={voice.isMuted}
                  onMuteToggle={voice.toggleMute}
                  onInterrupt={voice.interrupt}
                  availability={voice.availability}
                />
              </div>
            </div>
          </div>
        )}

        {isTicketModalOpen ? (
          <TicketModal onClose={() => setIsTicketModalOpen(false)} />
        ) : null}

      </div>
    </main>
  );
}

function TicketModal({ onClose }: { onClose: () => void }) {
  const [name, setName] = useState("");
  const [province, setProvince] = useState("");
  const [phone, setPhone] = useState("");
  const [email, setEmail] = useState("");
  const [content, setContent] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState("");
  const [createdId, setCreatedId] = useState("");

  const normalizedPhone = phone.trim();
  const normalizedEmail = email.trim();
  const canSubmit =
    province.trim().length > 0 &&
    /^\d{11}$/.test(normalizedPhone) &&
    content.trim().length >= 2 &&
    (!normalizedEmail || /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(normalizedEmail));

  async function handleTicketSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError("");
    if (!canSubmit) {
      setError("请检查省份、11 位手机号和咨询问题；邮箱如填写需为有效格式。");
      return;
    }
    setSubmitting(true);
    try {
      const ticket = await createPublicTicket({
        ...(name.trim() ? { name: name.trim() } : {}),
        province: province.trim(),
        phone: normalizedPhone,
        ...(normalizedEmail ? { email: normalizedEmail } : {}),
        content: content.trim()
      });
      setCreatedId(ticket.id);
      setName("");
      setProvince("");
      setPhone("");
      setEmail("");
      setContent("");
    } catch (error) {
      setError(error instanceof Error ? error.message : "留言提交失败，请稍后再试。");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="fixed inset-0 z-[70] flex items-center justify-center px-4 py-6">
      <div className="absolute inset-0 bg-slate-950/55 backdrop-blur-sm" onClick={onClose} />
      <section className="relative z-10 w-full max-w-[560px] overflow-hidden rounded-3xl border border-white/60 bg-white/95 shadow-2xl">
        <div className="flex items-start justify-between gap-4 border-b border-slate-100 px-6 py-5">
          <div>
            <h2 className="text-lg font-black text-school-900">留言咨询</h2>
            <p className="mt-1 text-sm leading-6 text-slate-500">
              留下联系方式和问题，招生咨询老师会在后台工单中看到并尽快处理。
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-full p-2 text-slate-400 transition hover:bg-slate-100 hover:text-slate-700"
            aria-label="关闭留言窗口"
          >
            <CloseIcon className="h-5 w-5" />
          </button>
        </div>

        {createdId ? (
          <div className="px-6 py-7">
            <div className="rounded-2xl border border-emerald-100 bg-emerald-50 px-4 py-4 text-sm leading-7 text-emerald-800">
              留言已提交，工单号：<span className="font-black">{createdId}</span>。请保持手机或邮箱可联系，老师会结合你的问题继续跟进。
            </div>
            <div className="mt-5 flex justify-end gap-3">
              <button
                type="button"
                onClick={() => setCreatedId("")}
                className="rounded-xl border border-slate-200 px-4 py-2 text-sm font-bold text-slate-600 transition hover:bg-slate-50"
              >
                再写一条
              </button>
              <button
                type="button"
                onClick={onClose}
                className="rounded-xl bg-school-900 px-4 py-2 text-sm font-bold text-white transition hover:bg-school-800"
              >
                完成
              </button>
            </div>
          </div>
        ) : (
          <form onSubmit={handleTicketSubmit} className="grid gap-4 px-6 py-6">
            <div className="grid gap-4 sm:grid-cols-2">
              <label className="grid gap-2 text-sm font-bold text-slate-700">
                姓名（可选）
                <input
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  maxLength={32}
                  placeholder="例如：张同学"
                  className="rounded-xl border border-slate-200 bg-white px-3 py-2.5 text-sm font-medium outline-none transition focus:border-school-500 focus:ring-4 focus:ring-school-100"
                />
              </label>
              <label className="grid gap-2 text-sm font-bold text-slate-700">
                省份
                <input
                  value={province}
                  onChange={(event) => setProvince(event.target.value)}
                  maxLength={32}
                  placeholder="例如：河北"
                  className="rounded-xl border border-slate-200 bg-white px-3 py-2.5 text-sm font-medium outline-none transition focus:border-school-500 focus:ring-4 focus:ring-school-100"
                  required
                />
              </label>
            </div>
            <div className="grid gap-4 sm:grid-cols-2">
              <label className="grid gap-2 text-sm font-bold text-slate-700">
                手机号
                <input
                  value={phone}
                  onChange={(event) => setPhone(event.target.value.replace(/\D/g, "").slice(0, 11))}
                  inputMode="numeric"
                  pattern="\d{11}"
                  placeholder="11 位手机号码"
                  className="rounded-xl border border-slate-200 bg-white px-3 py-2.5 text-sm font-medium outline-none transition focus:border-school-500 focus:ring-4 focus:ring-school-100"
                  required
                />
              </label>
              <label className="grid gap-2 text-sm font-bold text-slate-700">
                邮箱（可选）
                <input
                  value={email}
                  onChange={(event) => setEmail(event.target.value)}
                  type="email"
                  maxLength={120}
                  placeholder="用于接收补充回复"
                  className="rounded-xl border border-slate-200 bg-white px-3 py-2.5 text-sm font-medium outline-none transition focus:border-school-500 focus:ring-4 focus:ring-school-100"
                />
              </label>
            </div>
            <label className="grid gap-2 text-sm font-bold text-slate-700">
              咨询问题
              <textarea
                value={content}
                onChange={(event) => setContent(event.target.value.slice(0, 1000))}
                placeholder="请尽量写清省份、科类、分数、专业或想咨询的政策。"
                className="min-h-[132px] resize-none rounded-xl border border-slate-200 bg-white px-3 py-2.5 text-sm font-medium leading-6 outline-none transition focus:border-school-500 focus:ring-4 focus:ring-school-100"
                required
              />
            </label>
            {error ? (
              <div className="rounded-xl border border-rose-100 bg-rose-50 px-3 py-2 text-sm font-semibold text-rose-700">
                {error}
              </div>
            ) : null}
            <div className="flex flex-col-reverse gap-3 pt-1 sm:flex-row sm:justify-end">
              <button
                type="button"
                onClick={onClose}
                className="rounded-xl border border-slate-200 px-4 py-2.5 text-sm font-bold text-slate-600 transition hover:bg-slate-50"
              >
                取消
              </button>
              <button
                type="submit"
                disabled={submitting || !canSubmit}
                className="rounded-xl bg-school-900 px-5 py-2.5 text-sm font-black text-white shadow-sm transition hover:bg-school-800 disabled:cursor-not-allowed disabled:bg-slate-300"
              >
                {submitting ? "提交中..." : "提交留言"}
              </button>
            </div>
          </form>
        )}
      </section>
    </div>
  );
}

interface HeaderProps {
  onOpenDrawer?: () => void;
  selectedModel: string;
  onModelChange: (model: string) => void;
}

function Header({ onOpenDrawer, selectedModel, onModelChange }: HeaderProps) {
  return (
    <header className="relative z-20 flex h-[76px] shrink-0 items-center justify-between border-b border-slate-200/50 bg-white/70 px-4 shadow-[0_4px_30px_rgba(0,0,0,0.03)] backdrop-blur-md md:px-6">
      <div className="flex min-w-0 items-center gap-3">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center overflow-hidden rounded-full bg-white shadow-[0_4px_16px_rgba(8,23,46,0.08)] ring-2 ring-school-100">
          <img src="/coze-replica/hsul-logo.png" alt="哈尔滨师范大学校徽" className="h-9 w-9 object-contain" />
        </div>
        <div className="min-w-0">
          <h1 className="text-base font-extrabold leading-tight text-school-900 md:text-lg">
            哈尔滨师范大学
          </h1>
          <p className="text-[10px] md:text-xs font-semibold text-gold-600 tracking-wide mt-0.5">
            招生数字人智能问答系统
          </p>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-2 text-slate-600 md:gap-3">
        {/* Model Switcher Toggle */}
        <div className="flex items-center rounded-2xl bg-slate-100/80 p-0.5 border border-slate-200/40 shadow-inner backdrop-blur-sm mr-1">
          <button
            type="button"
            onClick={() => onModelChange("ours")}
            className={cn(
              "flex items-center gap-1.5 rounded-2xl px-3 py-1.5 text-xs font-bold transition-all duration-300",
              selectedModel === "ours"
                ? "bg-white text-school-900 shadow-soft scale-100"
                : "text-slate-500 hover:text-slate-900 scale-95"
            )}
          >
            <span className="relative flex h-2 w-2">
              <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"></span>
              <span className="relative inline-flex rounded-full h-2 w-2 bg-emerald-500"></span>
            </span>
            <span>我方自研</span>
          </button>
          <button
            type="button"
            onClick={() => onModelChange("theirs")}
            className={cn(
              "flex items-center gap-1.5 rounded-2xl px-3 py-1.5 text-xs font-bold transition-all duration-300",
              selectedModel === "theirs"
                ? "bg-white text-[#ce3459] shadow-soft scale-100"
                : "text-slate-500 hover:text-slate-900 scale-95"
            )}
          >
            <span className="relative flex h-2 w-2">
              <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-[#ce3459]/50 opacity-75"></span>
              <span className="relative inline-flex rounded-full h-2 w-2 bg-[#ce3459]"></span>
            </span>
            <span>对方竞品</span>
          </button>
        </div>

        {/* High Frequency Toggle for mobile/tablet */}
        <button
          type="button"
          onClick={onOpenDrawer}
          className="flex xl:hidden h-9 items-center gap-1.5 rounded-full bg-[#ce3459]/10 px-3.5 text-xs font-bold text-[#ce3459] hover:bg-[#ce3459]/20 transition"
        >
          <QuestionIcon className="h-4 w-4" />
          <span>高频咨询</span>
        </button>

        {/* WeChat official account hover popover */}
        <div className="group relative">
          <div className="flex cursor-pointer items-center gap-1.5 rounded-full bg-slate-100/80 px-3.5 py-1.5 hover:bg-slate-200/80 transition">
            <img src="/coze-replica/qrcode.jpg" alt="扫码关注官微" className="h-5 w-5 rounded object-cover" />
            <span className="hidden text-xs font-bold sm:inline text-slate-700">官方微信</span>
          </div>
          {/* Popover */}
          <div className="invisible absolute right-0 top-full z-40 mt-3 w-[210px] rounded-2xl border border-slate-200 bg-white p-4 opacity-0 shadow-xl transition-all duration-300 origin-top-right group-hover:visible group-hover:opacity-100 scale-95 group-hover:scale-100">
            <img src="/coze-replica/qrcode.jpg" alt="哈师大招生官方微信" className="h-[176px] w-full rounded-xl object-cover" />
            <p className="mt-2.5 text-center text-xs font-bold text-school-800">扫码关注哈师大官方微信</p>
            <p className="mt-1 text-center text-[10px] text-slate-400">实时获取最新招生简章</p>
          </div>
        </div>

        {/* Fullscreen Button */}
        <button
          type="button"
          title="全屏切换"
          onClick={() => {
            if (document.fullscreenElement) {
              void document.exitFullscreen();
            } else {
              void document.documentElement.requestFullscreen();
            }
          }}
          className="flex h-9 w-9 items-center justify-center rounded-full bg-slate-100/80 text-slate-500 hover:bg-slate-200/80 hover:text-slate-800 transition"
        >
          <MaximizeIcon className="h-4 w-4" />
        </button>
      </div>
    </header>
  );
}

function DigitalHumanStage({
  isSpeaking,
  stateLabel,
  isMuted,
  onMuteToggle,
  onInterrupt,
  availability
}: {
  isSpeaking: boolean;
  stateLabel: string;
  isMuted: boolean;
  onMuteToggle: () => void;
  onInterrupt: () => void;
  availability: string;
}) {
  return (
    <div className="dh-root flex-1 min-h-0 flex flex-col relative select-none">
      {/* Video/GIF Background */}
      <div className="dh-video-bg absolute inset-0">
        <img
          src="/coze-replica/muyang-listen.gif"
          alt="沐阳学长数字人"
          className="w-full h-full object-cover object-top"
        />
        {/* Subtle vignette overlay */}
        <div className="dh-vignette" />
      </div>

      {/* Top Banner Info */}
      <div className="absolute inset-x-0 top-0 z-10 flex items-center justify-between px-4 pt-4">
        <div className="rounded-2xl border border-white/10 bg-slate-950/40 px-3.5 py-1.5 backdrop-blur-xl">
          <span className="text-[10px] font-extrabold tracking-[0.1em] text-gold-300 block text-left">招生数字顾问</span>
          <div className="flex items-center gap-1.5 mt-0.5">
            <span className="text-sm font-extrabold text-white">沐阳学长</span>
            <span className="text-[9px] font-bold text-white bg-[#ce3459] px-1 rounded-md">AI 在线</span>
          </div>
        </div>

        <span className={cn(
          "inline-flex items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-semibold backdrop-blur-xl ring-1",
          availability === "unavailable" 
            ? "bg-rose-500/15 text-rose-200 ring-rose-300/20" 
            : "bg-slate-950/40 text-white ring-white/10"
        )}>
          <span className={cn(
            "h-1.5 w-1.5 rounded-full",
            availability === "unavailable" 
              ? "bg-rose-400" 
              : availability === "checking" 
                ? "bg-amber-400 animate-pulse" 
                : isSpeaking 
                  ? "bg-emerald-400 animate-pulse" 
                  : "bg-emerald-400"
          )} />
          <span className="text-[10px] text-white/90">{stateLabel}</span>
        </span>
      </div>

      {/* Centered Avatar Badge in bottom center */}
      <div className="absolute inset-x-0 bottom-16 flex flex-col items-center gap-2 pointer-events-none">
        {/* Breathing glow nameplate when speaking */}
        <div className={cn(
          "rounded-full bg-slate-950/60 border border-white/10 px-4 py-1.5 shadow-lg backdrop-blur-md text-center transition-all duration-300",
          isSpeaking && "avatar-breath-speak bg-[#ce3459]/20 border-[#ce3459]/30"
        )}>
          <span className="text-[11px] font-bold text-white tracking-wide">
            {isSpeaking ? "正在进行智能招生解答..." : "哈师大报考指南为您服务"}
          </span>
        </div>
      </div>

      {/* Speaking waveform overlay */}
      {isSpeaking && (
        <div className="dh-wave-strip absolute bottom-16">
          {Array.from({ length: 24 }).map((_, i) => (
            <span
              key={i}
              className="dh-wave-tick bg-gold-400"
              style={{
                animationDelay: `${i * 0.04}s`,
              }}
            />
          ))}
        </div>
      )}

      {/* Bottom glass control bar */}
      <div className="dh-overlay-bar absolute bottom-0 left-0 right-0 z-10">
        <div className="flex items-center gap-2">
          <div className="dh-avatar-badge bg-[#ce3459]/10 border-[#ce3459]/20 text-[#ce3459] w-8 h-8 rounded-lg flex items-center justify-center">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
              <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
              <path d="M19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07" />
            </svg>
          </div>
          <span className="text-[11px] font-semibold text-slate-300">播报设置</span>
        </div>

        <div className="dh-controls">
          {/* Mute button */}
          <button
            type="button"
            onClick={onMuteToggle}
            className={cn("dh-ctrl-btn", isMuted && "dh-ctrl-active bg-[#ce3459]/20 border-[#ce3459]/30 text-[#ce3459]")}
            title={isMuted ? "取消静音" : "静音"}
          >
            {isMuted ? (
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
                <line x1="23" y1="9" x2="17" y2="15" />
                <line x1="17" y1="9" x2="23" y2="15" />
              </svg>
            ) : (
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
                <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
              </svg>
            )}
          </button>

          {/* Interrupt button */}
          {isSpeaking && (
            <button
              type="button"
              onClick={onInterrupt}
              className="dh-ctrl-btn dh-ctrl-stop bg-rose-500/10 border-rose-500/20 text-rose-400 hover:bg-rose-500/20"
              title="打断播报"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <rect x="6" y="4" width="4" height="16" rx="1" />
                <rect x="14" y="4" width="4" height="16" rx="1" />
              </svg>
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function QuickAccess() {
  const featuredLinks = quickLinks.filter((item) => item.featured);
  const normalLinks = quickLinks.filter((item) => !item.featured);

  return (
    <div className="flex flex-col gap-3">
      {/* Featured Links Grid */}
      <div className="grid grid-cols-2 gap-2.5">
        {featuredLinks.map((item) => (
          <QuickLink key={item.label} item={item} featured />
        ))}
      </div>
      {/* Normal Links Grid */}
      <div className="grid grid-cols-2 gap-2">
        {normalLinks.map((item) => (
          <QuickLink key={item.label} item={item} />
        ))}
      </div>
    </div>
  );
}

function QuickLink({ item, featured = false }: { item: (typeof quickLinks)[number]; featured?: boolean }) {
  const Icon = item.icon;

  return (
    <a
      href={item.href}
      target="_blank"
      rel="noopener noreferrer"
      className={cn(
        "flex items-center justify-center transition-all duration-300 hover-zoom",
        featured
          ? "gap-2 rounded-2xl bg-gradient-to-br from-[#ce3459]/10 to-[#ce3459]/5 border border-[#ce3459]/20 px-3.5 py-2.5 text-xs font-extrabold text-[#ce3459] shadow-sm hover:from-[#ce3459]/15 hover:to-[#ce3459]/10"
          : "gap-2 rounded-xl bg-slate-50 border border-slate-200/50 px-3 py-2 text-xs font-bold text-slate-600 hover:bg-school-50 hover:text-school-700 hover:border-school-200"
      )}
    >
      <Icon className={cn("shrink-0", featured ? "h-4 w-4" : "h-3.5 w-3.5")} />
      <span>{item.label}</span>
    </a>
  );
}

function HotQuestions({ onAsk }: { onAsk: (question: string) => void | Promise<void> }) {
  const [activeCategory, setActiveCategory] = useState<string>(defaultHotCategory);
  const [batchIndex, setBatchIndex] = useState(0);
  const activeGroups = hotQuestionGroups[activeCategory] ?? hotQuestionGroups[defaultHotCategory]!;
  const hotQuestions = useMemo(() => {
    return activeGroups[batchIndex % activeGroups.length] ?? activeGroups[0]!;
  }, [activeGroups, batchIndex]);

  return (
    <div className="flex flex-col h-full text-left">
      {/* Sidebar Header */}
      <div className="mb-4 flex items-center justify-between">
        <h2 className="flex items-center gap-2 text-sm font-extrabold text-school-900">
          <span className="h-4 w-1 rounded bg-[#ce3459]" />
          高频热门咨询
        </h2>
        <button
          type="button"
          onClick={() => setBatchIndex((current) => current + 1)}
          className="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-bold text-slate-500 hover:bg-slate-100 hover:text-slate-800 transition"
        >
          <RefreshIcon className="h-3.5 w-3.5" />
          <span>换一批</span>
        </button>
      </div>

      {/* Categories Grid (2 Columns) */}
      <div className="mb-4 grid grid-cols-2 gap-1.5">
        {hotCategories.map((category) => {
          const Icon = category.icon;
          const isActive = activeCategory === category.label;
          return (
            <button
              key={category.label}
              type="button"
              onClick={() => {
                setActiveCategory(category.label);
                setBatchIndex(0);
              }}
              className={cn(
                "inline-flex h-9 items-center gap-1.5 rounded-xl px-2.5 text-[11px] font-extrabold transition-all duration-200 border",
                isActive
                  ? "bg-[#ce3459] text-white border-[#ce3459] shadow-sm"
                  : "bg-slate-50 text-slate-600 border-slate-200/60 hover:bg-slate-100"
              )}
            >
              <Icon className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{category.label}</span>
            </button>
          );
        })}
      </div>

      {/* Questions List */}
      <div className="flex-1 flex flex-col gap-2 overflow-y-auto pr-0.5 custom-scrollbar">
        {hotQuestions.map((question, index) => (
          <button
            key={question}
            type="button"
            onClick={() => {
              void onAsk(question);
            }}
            className="group rounded-2xl border border-slate-200/50 bg-white px-3.5 py-3 text-left text-xs font-bold leading-normal text-slate-700 shadow-sm transition-all duration-300 hover:border-[#ce3459]/30 hover:bg-gradient-to-r hover:from-white hover:to-[#ce3459]/5 hover:shadow-md hover:translate-x-1"
          >
            <div className="flex gap-2">
              <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-slate-100 text-[10px] font-bold text-slate-400 group-hover:bg-[#ce3459]/10 group-hover:text-[#ce3459] transition-colors">
                {index + 1}
              </span>
              <span className="font-sans font-semibold tracking-wide text-slate-700 group-hover:text-school-900 transition-colors">
                {question}
              </span>
            </div>
          </button>
        ))}
      </div>
    </div>
  );
}

function WelcomeBubble({
  isSpeaking,
  latestAssistant,
  onReplay
}: {
  isSpeaking: boolean;
  latestAssistant: string | null;
  onReplay: () => void;
}) {
  return (
    <div className="flex items-start gap-4">
      <div className="mt-1 h-10 w-10 shrink-0 overflow-hidden rounded-full border-2 border-white bg-gradient-to-br from-school-300 to-school-600 shadow-md">
        <img src="/coze-replica/muyang-listen.gif" alt="沐阳头像" className="h-full w-full object-cover object-top" />
      </div>
      <div className="max-w-[min(680px,calc(100%-56px))] rounded-2xl rounded-tl-none border border-school-100 bg-school-50/50 p-4 shadow-sm text-left">
        <p className="text-sm font-medium leading-7 text-school-950">
          哈喽～我是哈师大招生小助手沐阳😊 不管是录取分数、位次参考、各专业情况、公费师范政策、校园生活、就业升学，还是想做志愿位次测评，都可以随时问我，我来帮你一一解答！
        </p>
        <p className="mt-3 text-[10px] font-medium text-slate-400">内容由 AI 生成，仅供参考，具体请以院校官方公告为准。</p>
        
        <div className="mt-3.5 flex flex-wrap items-center justify-between gap-3 border-t border-slate-200/50 pt-2.5 text-xs text-slate-500">
          <div className="flex items-center gap-3">
            <span>官方服务</span>
            <span className="h-3 w-[1px] bg-slate-300" />
            <button 
              type="button"
              onClick={() => {
                navigator.clipboard.writeText("哈师大招生小助手沐阳").then(() => alert("已复制名称"));
              }}
              className="hover:text-slate-800 flex items-center gap-1"
              title="复制名称"
            >
              <CopyIcon className="h-3.5 w-3.5" />
              <span>复制</span>
            </button>
          </div>
          <button
            type="button"
            disabled={!latestAssistant && !isSpeaking}
            onClick={onReplay}
            className="inline-flex items-center gap-1.5 rounded-lg bg-white border border-slate-200 px-3 py-1.5 text-xs font-bold text-slate-700 shadow-sm transition hover:bg-slate-50 hover:text-school-800 disabled:opacity-50 disabled:pointer-events-none"
          >
            <SpeakerIcon className="h-3.5 w-3.5" />
            播放语音
          </button>
        </div>
      </div>
    </div>
  );
}

function ChatBubble({ children, role }: { children: ReactNode; role: "user" | "assistant" }) {
  const isUser = role === "user";

  return (
    <div className={cn("flex w-full items-start gap-3.5", isUser ? "justify-end" : "justify-start")}>
      {!isUser ? (
        <div className="mt-1 h-10 w-10 shrink-0 overflow-hidden rounded-full border-2 border-white bg-gradient-to-br from-school-300 to-school-600 shadow-md">
          <img src="/coze-replica/muyang-listen.gif" alt="沐阳头像" className="h-full w-full object-cover object-top" />
        </div>
      ) : null}
      <div
        className={cn(
          "rounded-2xl p-4 text-sm leading-7 shadow-sm border font-medium text-left",
          isUser 
            ? "max-w-[82%] rounded-tr-none bg-gradient-to-br from-[#ce3459] to-[#b02244] text-white border-[#ce3459]/20" 
            : "max-w-[min(680px,calc(100%-56px))] rounded-tl-none border-slate-200/60 bg-white text-slate-800"
        )}
      >
        {typeof children === "string" ? (
          <FormattedMessage className="font-sans tracking-wide" text={children} />
        ) : (
          <p className="whitespace-pre-wrap font-sans tracking-wide">{children}</p>
        )}
      </div>
      {isUser ? (
        <div className="mt-1 h-10 w-10 shrink-0 overflow-hidden rounded-full border-2 border-white bg-gradient-to-br from-[#ce3459] to-[#b02244] shadow-md flex items-center justify-center text-white text-[10px] font-bold">
          <span>考生</span>
        </div>
      ) : null}
    </div>
  );
}

function AssistantStatusLine({ text }: { text: string }) {
  return (
    <div className="flex w-full items-start gap-3.5">
      <div className="mt-1 h-10 w-10 shrink-0 overflow-hidden rounded-full border-2 border-white bg-gradient-to-br from-school-300 to-school-600 shadow-md">
        <img src="/coze-replica/muyang-listen.gif" alt="沐阳头像" className="h-full w-full object-cover object-top" />
      </div>
      <div className="rounded-full border border-slate-200/70 bg-white/80 px-4 py-2 text-xs font-semibold text-slate-500 shadow-sm">
        {text}
      </div>
    </div>
  );
}

function RichAnswerBubble({ message }: { message: Extract<LocalRichMessage, { role: "assistant" }> }) {
  return (
    <div className="flex items-start gap-3.5">
      <div className="mt-1 h-10 w-10 shrink-0 overflow-hidden rounded-full border-2 border-white bg-gradient-to-br from-school-300 to-school-600 shadow-md">
        <img src="/coze-replica/muyang-listen.gif" alt="沐阳头像" className="h-full w-full object-cover object-top" />
      </div>
      <div className="max-w-[min(720px,calc(100%-56px))] rounded-2xl rounded-tl-none border border-slate-200/60 bg-white p-5 shadow-sm text-left">
        <FormattedMessage className="text-sm leading-8 font-medium text-slate-800" text={message.content} />

        {message.video ? (
          <div className="mt-4 overflow-hidden rounded-2xl border border-slate-200 bg-slate-950 shadow-md hover-zoom relative group">
            <video
              controls
              preload="metadata"
              src={message.video.src}
              title={message.video.title}
              className="max-h-[300px] w-full object-cover"
            />
            {/* Elegant overlay ribbon */}
            <div className="absolute top-3 left-3 bg-slate-900/80 text-gold-300 text-[10px] font-bold px-2.5 py-1 rounded-full border border-white/10 backdrop-blur">
              {message.video.title || "宣传片"}
            </div>
          </div>
        ) : null}

        {message.images?.length ? (
          <div className="mt-4 grid grid-cols-2 gap-3">
            {message.images.map((image, index) => (
              <div 
                key={image.src}
                className={cn(
                  "overflow-hidden rounded-2xl border border-slate-200/60 shadow-sm hover-zoom",
                  message.images!.length === 5 && index === 0 ? "col-span-2 h-48" : "h-32"
                )}
              >
                <img
                  src={image.src}
                  alt={image.alt}
                  className="h-full w-full object-cover hover:scale-105 transition-transform duration-500"
                  loading="lazy"
                />
              </div>
            ))}
          </div>
        ) : null}

        <p className="mt-4 text-[10px] font-medium text-slate-400">内容由 AI 生成，仅供参考，具体请以院校官方公告为准。</p>
        
        <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-slate-100 pt-3 text-[11px] font-medium text-slate-500">
          <div className="flex items-center gap-3">
            <span>{new Date().toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}</span>
            <span className="h-3 w-[1px] bg-slate-300" />
            <button 
              type="button" 
              onClick={() => {
                navigator.clipboard.writeText(message.content).then(() => alert("已复制回答内容"));
              }}
              className="hover:text-slate-800 flex items-center gap-1"
            >
              <CopyIcon className="h-3.5 w-3.5" />
              <span>复制内容</span>
            </button>
          </div>
          <span className="inline-flex items-center gap-1.5 text-emerald-600">
            <span className="h-2 w-2 bg-emerald-500 rounded-full animate-ping" />
            <span>智能播报</span>
          </span>
        </div>
      </div>
    </div>
  );
}

function InputDock({
  disabled,
  input,
  onAsk,
  onInputChange,
  onInterrupt,
  onClear,
  onSubmit,
  onOpenDrawer,
  onOpenTicket
}: {
  disabled: boolean;
  input: string;
  onAsk: (question: string) => void | Promise<void>;
  onInputChange: (value: string) => void;
  onInterrupt: () => void;
  onClear: () => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void | Promise<void>;
  onOpenDrawer: () => void;
  onOpenTicket: () => void;
}) {
  return (
    <div className="shrink-0 border-t border-slate-200/50 bg-white/60 px-4 py-4 backdrop-blur-md sm:px-6 lg:px-8 relative z-10">
      <div className="mx-auto w-full max-w-[800px]">
        {/* Tool buttons */}
        <div className="mb-3 flex items-center justify-between gap-4 overflow-x-auto pb-1 custom-scrollbar">
          <div className="flex shrink-0 items-center gap-2">
            <ToolButton active icon={ClipboardIcon} label="志愿测评" onClick={() => void onAsk("我想做分步志愿测评，请先问我需要提供哪些信息。")} />
            <ToolButton icon={QuestionIcon} label="常见问题" onClick={() => void onAsk("请列出哈尔滨师范大学招生咨询的常见问题。")} />
            <ToolButton icon={MessageIcon} label="留言咨询" onClick={onOpenTicket} />
            <button
              type="button"
              onClick={onOpenDrawer}
              className="xl:hidden inline-flex h-8 items-center gap-1.5 rounded-lg border border-slate-200/50 px-3 text-[11px] font-extrabold text-slate-500 hover:bg-slate-100 hover:text-slate-800 transition"
            >
              <QuestionIcon className="h-3.5 w-3.5 animate-bounce" />
              <span>高频问题</span>
            </button>
            <ToolButton icon={TrashIcon} label="清空记录" onClick={onClear} />
          </div>
          <p className="hidden shrink-0 text-[10px] font-extrabold text-slate-400 sm:block">Enter 发送 / Shift+Enter 换行</p>
        </div>

        {/* Input box form */}
        <form onSubmit={onSubmit} className="flex items-end gap-2.5">
          {/* Audio toggle/interrupt */}
          <button
            type="button"
            title="打断语音播报"
            onClick={onInterrupt}
            className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border border-slate-200 bg-white text-slate-500 transition hover:bg-[#ce3459]/5 hover:text-[#ce3459] hover:border-[#ce3459]/30"
          >
            <MicIcon className="h-5 w-5" />
          </button>

          {/* Textarea input */}
          <div className="min-w-0 flex-1">
            <label htmlFor="chat-input" className="sr-only">请输入您的报考问题</label>
            <textarea
              id="chat-input"
              value={input}
              rows={1}
              onChange={(event) => onInputChange(event.target.value)}
              onInput={(event) => {
                const target = event.currentTarget;
                target.style.height = "44px";
                target.style.height = `${Math.min(target.scrollHeight, 120)}px`;
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault();
                  if (!disabled && input.trim()) {
                    void onAsk(input);
                  }
                }
              }}
              placeholder="请输入您的报考问题，如：哈师大有哪些王牌专业？"
              className="max-h-[120px] min-h-[44px] w-full resize-none rounded-2xl border border-slate-200 bg-white px-4 py-3 text-sm font-semibold leading-5 text-slate-800 shadow-sm outline-none transition placeholder:text-slate-400 focus:border-[#ce3459]/40 focus:bg-white focus:ring-4 focus:ring-[#ce3459]/5"
            />
          </div>

          {/* Send button */}
          <button
            type="submit"
            disabled={disabled || !input.trim()}
            aria-label="发送"
            className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl bg-[#ce3459] text-white shadow-sm transition hover:bg-[#b02244] disabled:cursor-not-allowed disabled:bg-slate-200 disabled:text-slate-400"
          >
            <SendIcon className="h-5 w-5" />
          </button>
        </form>
      </div>
    </div>
  );
}

function ToolButton({
  active = false,
  icon: Icon,
  label,
  onClick
}: {
  active?: boolean;
  icon: IconComponent;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "inline-flex h-8 items-center gap-1.5 rounded-lg px-3 text-[11px] font-extrabold transition border",
        active 
          ? "border-[#ce3459]/20 bg-[#ce3459]/10 text-[#ce3459]" 
          : "border-slate-200/50 text-slate-500 hover:bg-slate-100 hover:text-slate-800"
      )}
    >
      <Icon className="h-3.5 w-3.5" />
      <span>{label}</span>
    </button>
  );
}

type IconComponent = ({ className }: { className?: string }) => ReactNode;

function Svg({ className, children }: { className?: string | undefined; children: ReactNode }) {
  return (
    <svg className={className} fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" viewBox="0 0 24 24">
      {children}
    </svg>
  );
}

function ClipboardIcon({ className }: { className?: string }) {
  return <Svg className={className}><rect height="4" rx="1" width="8" x="8" y="2" /><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" /><path d="M12 11h4M12 16h4M8 11h.01M8 16h.01" /></Svg>;
}

function TrendIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="m22 7-8.5 8.5-5-5L2 17" /><path d="M16 7h6v6" /></Svg>;
}

function FileIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><path d="M14 2v6h6M8 13h8M8 17h6" /></Svg>;
}

function ScaleIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="m16 16 3-8 3 8c-.9 1.3-5.1 1.3-6 0ZM2 16l3-8 3 8c-.9 1.3-5.1 1.3-6 0ZM7 21h10M12 3v18M3 7h18" /></Svg>;
}

function RibbonIcon({ className }: { className?: string }) {
  return <Svg className={className}><circle cx="12" cy="8" r="5" /><path d="M8.5 12.5 7 22l5-3 5 3-1.5-9.5" /></Svg>;
}

function BuildingIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="M3 21h18M5 21V7l8-4v18M19 21V11l-6-4M9 9h.01M9 13h.01M9 17h.01M15 13h.01M15 17h.01" /></Svg>;
}

function GlobeIcon({ className }: { className?: string }) {
  return <Svg className={className}><circle cx="12" cy="12" r="10" /><path d="M12 2a14.5 14.5 0 0 0 0 20M2 12h20" /></Svg>;
}

function BookIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="M12 7v14" /><path d="M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z" /></Svg>;
}

function MicroscopeIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="M6 18h8M3 22h18M14 22a7 7 0 1 0 0-14h-1M9 14h2M9 12a2 2 0 0 1-2-2V6h6v4a2 2 0 0 1-2 2ZM12 6V3a1 1 0 0 0-1-1H9a1 1 0 0 0-1 1v3" /></Svg>;
}

function NewsIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="M4 22h16a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2H8a2 2 0 0 0-2 2v16a2 2 0 0 1-2 2Zm0 0a2 2 0 0 1-2-2v-9c0-1.1.9-2 2-2h2" /><path d="M18 14h-8M15 18h-5M10 6h8v4h-8z" /></Svg>;
}

function RefreshIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="M3 12a9 9 0 0 1 15.5-6.2L21 8" /><path d="M21 3v5h-5M21 12a9 9 0 0 1-15.5 6.2L3 16" /><path d="M3 21v-5h5" /></Svg>;
}

function MaximizeIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" /></Svg>;
}

function CopyIcon({ className }: { className?: string }) {
  return <Svg className={className}><rect height="14" rx="2" width="14" x="8" y="8" /><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" /></Svg>;
}

function SpeakerIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="M11 5 6 9H2v6h4l5 4zM15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14" /></Svg>;
}

function QuestionIcon({ className }: { className?: string }) {
  return <Svg className={className}><circle cx="12" cy="12" r="10" /><path d="M9.1 9a3 3 0 0 1 5.8 1c0 2-3 3-3 3M12 17h.01" /></Svg>;
}

function MessageIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" /><path d="M13 8H7M17 12H7" /></Svg>;
}

function TrashIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="M3 6h18M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2M10 11v6M14 11v6" /></Svg>;
}

function MicIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3ZM19 10v2a7 7 0 0 1-14 0v-2M12 19v3" /></Svg>;
}

function SendIcon({ className }: { className?: string }) {
  return <Svg className={className}><path d="m22 2-7 20-4-9-9-4Z" /><path d="M22 2 11 13" /></Svg>;
}

function CloseIcon({ className }: { className?: string }) {
  return <Svg className={className}><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></Svg>;
}
