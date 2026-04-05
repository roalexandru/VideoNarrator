import type { NarrationStyleId, Language, AiProvider, ModelId, TtsProvider } from "../types/config";
import type { ExportFormat } from "../types/export";

export const STYLES: {
  id: NarrationStyleId;
  label: string;
  description: string;
  icon: string;
}[] = [
  {
    id: "executive",
    label: "Executive Overview",
    description:
      "Confident, outcome-focused, minimal jargon. Business value and ROI.",
    icon: "briefcase",
  },
  {
    id: "product_demo",
    label: "Product Demo",
    description:
      'Polished walkthrough for customers. "You can" framing.',
    icon: "play",
  },
  {
    id: "technical",
    label: "Technical Deep-Dive",
    description:
      "Precise, developer-oriented. Names APIs and config options.",
    icon: "code",
  },
  {
    id: "teaser",
    label: "Teaser / Trailer",
    description: "High-energy, punchy sentences. Wow moments.",
    icon: "zap",
  },
  {
    id: "training",
    label: "Training Walkthrough",
    description:
      'Patient, methodical. "First we\'ll...", "Notice how...".',
    icon: "book",
  },
  {
    id: "critique",
    label: "Bug Review / Critique",
    description:
      "Analytical review. Identifies issues, UX problems, improvements.",
    icon: "search",
  },
];

export const LANGUAGES: Language[] = [
  { code: "en", label: "English", flag: "🇺🇸" },
  { code: "ja", label: "Japanese", flag: "🇯🇵" },
  { code: "de", label: "German", flag: "🇩🇪" },
  { code: "fr", label: "French", flag: "🇫🇷" },
  { code: "pt-BR", label: "Portuguese (BR)", flag: "🇧🇷" },
];

export const PROVIDERS: {
  id: AiProvider;
  label: string;
  models: { id: ModelId; label: string }[];
}[] = [
  {
    id: "claude",
    label: "Anthropic (Claude)",
    models: [
      { id: "claude-sonnet-4-20250514", label: "Claude Sonnet 4" },
      { id: "claude-opus-4-20250514", label: "Claude Opus 4" },
    ],
  },
  {
    id: "openai",
    label: "OpenAI",
    models: [
      { id: "gpt-4o", label: "GPT-4o" },
      { id: "o3", label: "o3" },
    ],
  },
  {
    id: "gemini",
    label: "Google (Gemini)",
    models: [
      { id: "gemini-2.5-flash", label: "Gemini 2.5 Flash" },
      { id: "gemini-2.5-pro", label: "Gemini 2.5 Pro" },
    ],
  },
];

export const ELEVEN_MODELS: { id: string; label: string }[] = [
  { id: "eleven_multilingual_v2", label: "Multilingual v2" },
  { id: "eleven_flash_v2_5", label: "Flash v2.5" },
  { id: "eleven_turbo_v2_5", label: "Turbo v2.5" },
  { id: "eleven_v3", label: "v3" },
];

export const TTS_PROVIDERS: {
  id: TtsProvider;
  label: string;
  description: string;
}[] = [
  {
    id: "elevenlabs",
    label: "ElevenLabs",
    description: "Premium voice synthesis with cloning",
  },
  {
    id: "azure",
    label: "Azure TTS",
    description: "Microsoft neural voices with narration styles",
  },
];

export const EXPORT_FORMATS: { id: ExportFormat; label: string; ext: string }[] = [
  { id: "json", label: "JSON (Structured)", ext: ".json" },
  { id: "srt", label: "SRT (Subtitles)", ext: ".srt" },
  { id: "vtt", label: "WebVTT", ext: ".vtt" },
  { id: "txt", label: "Plain Text", ext: ".txt" },
  { id: "md", label: "Markdown", ext: ".md" },
  { id: "ssml", label: "SSML (Speech)", ext: ".ssml" },
];
