import type { DictationSession, Engine, VocabularyEntry } from "../types";

export const weeklyWords = [
  { day: "Mon", words: 2640 },
  { day: "Tue", words: 3380 },
  { day: "Wed", words: 2860 },
  { day: "Thu", words: 4230 },
  { day: "Fri", words: 3690 },
  { day: "Sat", words: 1760 },
  { day: "Sun", words: 3120 },
];

export const languages = [
  { name: "English", code: "EN", percentage: 68, color: "#5b5ce2" },
  { name: "Hindi", code: "HI", percentage: 22, color: "#9a91f2" },
  { name: "Bengali", code: "BN", percentage: 10, color: "#d8d5fb" },
];

export const initialEngines: Engine[] = [
  {
    id: "whisper-turbo",
    name: "Whisper large-v3-turbo",
    provider: "whisper.cpp",
    description: "Fast, accurate multilingual transcription on your Mac.",
    kind: "local",
    status: "available",
    languageSupport: "Multilingual model",
    size: "1.6 GB",
    performance: "Benchmark pending",
    recommended: true,
  },
  {
    id: "whisper-small",
    name: "Whisper small",
    provider: "whisper.cpp",
    description: "A lightweight model for older hardware and quick notes.",
    kind: "local",
    status: "available",
    languageSupport: "Multilingual model",
    size: "466 MB",
    performance: "Benchmark pending",
  },
  {
    id: "whisper-base",
    name: "Whisper base",
    provider: "whisper.cpp",
    description: "A compact multilingual model for short everyday dictation.",
    kind: "local",
    status: "available",
    languageSupport: "Multilingual model",
    size: "142 MB",
    performance: "Benchmark pending",
  },
  {
    id: "openai-transcribe",
    name: "GPT-4o Transcribe",
    provider: "OpenAI",
    description: "Planned adapter for compatible OpenAI speech models.",
    kind: "cloud",
    status: "available",
    languageSupport: "Model-dependent",
    performance: "Adapter planned",
  },
  {
    id: "gemini-live",
    name: "Gemini Live",
    provider: "Google",
    description: "Planned adapter for compatible Gemini speech models.",
    kind: "cloud",
    status: "available",
    languageSupport: "Model-dependent",
    performance: "Adapter planned",
  },
  {
    id: "grok-voice",
    name: "Grok Voice",
    provider: "xAI",
    description:
      "Planned adapter whose capabilities will be enabled when verified.",
    kind: "cloud",
    status: "available",
    languageSupport: "To be verified",
    performance: "Adapter planned",
  },
];

export const initialVocabulary: VocabularyEntry[] = [
  {
    id: "1",
    phrase: "Spick",
    soundsLike: "speak",
    category: "Company",
    language: "English",
  },
  {
    id: "2",
    phrase: "whisper.cpp",
    soundsLike: "whisper dot C P P",
    category: "Technical",
    language: "English",
  },
  {
    id: "3",
    phrase: "Tauri",
    soundsLike: "tow-ree",
    category: "Technical",
    language: "English",
  },
  {
    id: "4",
    phrase: "Kubernetes",
    soundsLike: "koo-ber-net-eez",
    category: "Technical",
    language: "English",
  },
  {
    id: "5",
    phrase: "LLM",
    soundsLike: "L L M",
    category: "Technical",
    language: "English",
  },
  {
    id: "6",
    phrase: "artificial intelligence",
    soundsLike: "AI",
    category: "Replacement",
    language: "English",
  },
];

export const recentDictations: DictationSession[] = [
  {
    id: "d1",
    application: "Notion",
    text: "Let’s keep the onboarding focused on one clear promise: speak naturally, then get polished text anywhere.",
    timestamp: "12 min ago",
    words: 18,
    language: "EN",
    color: "#121212",
  },
  {
    id: "d2",
    application: "Slack",
    text: "Sounds good. I’ll have the multilingual model comparison ready before our design review tomorrow.",
    timestamp: "48 min ago",
    words: 15,
    language: "EN",
    color: "#5b5ce2",
  },
  {
    id: "d3",
    application: "VS Code",
    text: "Add a provider adapter that exposes streaming, language hints, translation, and vocabulary support.",
    timestamp: "2 hr ago",
    words: 14,
    language: "EN",
    color: "#2478d2",
  },
];
