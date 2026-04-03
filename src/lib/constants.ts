import type { NarrationStyleId, Language, AiProvider, ModelId } from "../types/config";
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
];

export const EXPORT_FORMATS: { id: ExportFormat; label: string; ext: string }[] = [
  { id: "json", label: "JSON (Structured)", ext: ".json" },
  { id: "srt", label: "SRT (Subtitles)", ext: ".srt" },
  { id: "vtt", label: "WebVTT", ext: ".vtt" },
  { id: "txt", label: "Plain Text", ext: ".txt" },
  { id: "md", label: "Markdown", ext: ".md" },
  { id: "ssml", label: "SSML (Speech)", ext: ".ssml" },
];
