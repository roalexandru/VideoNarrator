import type { NarrationStyleId, Language, AiProvider, CurrentModelId, ReasoningEffort, TtsProvider } from "../types/config";
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

/**
 * Model picker catalog — current generation only.
 *
 * `hint` is shown next to the label. Narration sends every sampled frame as an
 * image, so per-token cost scales with video length far faster than in a
 * text-only app; the hints exist so the tier tradeoff is visible at the point of
 * choice rather than discovered on the bill.
 *
 * Ordered cheapest-capable first within each provider, with the recommended
 * default listed first overall for that provider.
 */
export const PROVIDERS: {
  id: AiProvider;
  label: string;
  models: { id: CurrentModelId; label: string; hint?: string }[];
}[] = [
  {
    id: "claude",
    label: "Anthropic (Claude)",
    models: [
      { id: "claude-sonnet-5", label: "Claude Sonnet 5", hint: "Recommended — best speed/quality balance" },
      { id: "claude-opus-5", label: "Claude Opus 5", hint: "Deeper reasoning on complex footage" },
      { id: "claude-haiku-4-5", label: "Claude Haiku 4.5", hint: "Fastest and cheapest" },
      { id: "claude-fable-5", label: "Claude Fable 5", hint: "Most capable — premium pricing" },
    ],
  },
  {
    id: "openai",
    label: "OpenAI",
    models: [
      { id: "gpt-5.6-sol", label: "GPT-5.6 Sol", hint: "Recommended — the workhorse variant" },
      { id: "gpt-5.6-terra", label: "GPT-5.6 Terra", hint: "Mid-tier" },
      { id: "gpt-5.6-luna", label: "GPT-5.6 Luna", hint: "Budget variant" },
    ],
  },
  {
    id: "gemini",
    label: "Google (Gemini)",
    models: [
      { id: "gemini-3.6-flash", label: "Gemini 3.6 Flash", hint: "Recommended — strong multimodal, low cost" },
      { id: "gemini-3.5-flash", label: "Gemini 3.5 Flash" },
      { id: "gemini-3.5-flash-lite", label: "Gemini 3.5 Flash-Lite", hint: "Cheapest" },
      { id: "gemini-3.1-pro-preview", label: "Gemini 3.1 Pro", hint: "Most capable — preview" },
    ],
  },
];

/** Default model per provider — used when switching providers. */
export const DEFAULT_MODEL_FOR_PROVIDER: Record<AiProvider, CurrentModelId> = {
  claude: "claude-sonnet-5",
  openai: "gpt-5.6-sol",
  gemini: "gemini-3.6-flash",
};

/**
 * Reasoning-depth choices. Every current frontier model exposes a thinking
 * knob, but each vendor names it differently — one product-level choice is
 * mapped per provider in the Rust client (and clamped where a provider's ladder
 * is shorter, e.g. Gemini stops at `high`).
 */
export const REASONING_LEVELS: {
  id: ReasoningEffort;
  label: string;
  hint: string;
}[] = [
  { id: "fast", label: "Fast", hint: "Least thinking — quickest and cheapest" },
  { id: "balanced", label: "Balanced", hint: "Good default for most videos" },
  { id: "thorough", label: "Thorough", hint: "More analysis before writing" },
  { id: "max", label: "Maximum", hint: "Deepest reasoning — slowest and priciest" },
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
  {
    id: "builtin",
    label: "Built-in (Free)",
    description: "Uses your OS speech engine. No API key needed.",
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
