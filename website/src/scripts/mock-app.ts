// ─── Mock Data ───────────────────────────────────────────────

interface Segment {
  index: number;
  start_seconds: number;
  end_seconds: number;
  text: string;
  visual_description: string;
  pace: "slow" | "medium" | "fast";
}

const SEGMENTS: Segment[] = [
  {
    index: 0,
    start_seconds: 0,
    end_seconds: 8.5,
    text: "Welcome to Narrator — the AI-powered video narration tool that transforms raw footage into polished, professional presentations.",
    visual_description: "App logo animation with gradient background",
    pace: "medium",
  },
  {
    index: 1,
    start_seconds: 8.5,
    end_seconds: 18.2,
    text: "Simply drag and drop your video file, or use the built-in screen recorder to capture directly from your desktop.",
    visual_description: "User dragging a video file into the import area",
    pace: "medium",
  },
  {
    index: 2,
    start_seconds: 18.2,
    end_seconds: 29.0,
    text: "The video editor lets you trim, split, and reorder clips. Adjust playback speed or remove unwanted sections before generating narration.",
    visual_description: "Timeline editor with clips being rearranged",
    pace: "medium",
  },
  {
    index: 3,
    start_seconds: 29.0,
    end_seconds: 40.5,
    text: "Choose your preferred AI narration engine. Configure the style, tone, and language to match your audience perfectly.",
    visual_description: "Configuration panel with AI provider selection",
    pace: "slow",
  },
  {
    index: 4,
    start_seconds: 40.5,
    end_seconds: 52.0,
    text: "Watch as AI analyzes each frame of your video, generating perfectly timed narration segments with natural pacing and emphasis.",
    visual_description: "Processing screen with progress indicators",
    pace: "medium",
  },
  {
    index: 5,
    start_seconds: 52.0,
    end_seconds: 63.8,
    text: "Export your narration as subtitles, scripts, or full audio. Choose from SRT, VTT, JSON, Markdown, and more — or generate speech with ElevenLabs.",
    visual_description: "Export screen showing format options and audio generation",
    pace: "fast",
  },
];

const TOTAL_DURATION = 63.8;

// ─── State ───────────────────────────────────────────────────

let activeTab: "review" | "export" = "review";
let activeSegmentIndex = 0;
let isPlaying = false;
let currentTime = 0;
let playInterval: ReturnType<typeof setInterval> | null = null;

// ─── Helpers ─────────────────────────────────────────────────

function formatTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function getSegmentAtTime(time: number): number {
  for (let i = SEGMENTS.length - 1; i >= 0; i--) {
    if (time >= SEGMENTS[i].start_seconds) return i;
  }
  return 0;
}

// ─── DOM Refs ────────────────────────────────────────────────

function $(id: string): HTMLElement {
  return document.getElementById(id)!;
}

// ─── Tab Switching ───────────────────────────────────────────

const ICON_REVIEW = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>`;
const ICON_EXPORT = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>`;
const ICON_CHECK = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>`;

function switchTab(tab: "review" | "export") {
  if (tab === activeTab) return;
  activeTab = tab;

  const stepReview = $("mock-step-review");
  const stepExport = $("mock-step-export");
  const iconReview = $("mock-step-review-icon");
  const iconExport = $("mock-step-export-icon");
  const descReview = $("mock-step-review-desc");
  const descExport = $("mock-step-export-desc");

  if (tab === "review") {
    // Review: active
    stepReview.style.background = "rgba(99,102,241,0.12)";
    stepReview.style.color = "#a5b4fc";
    stepReview.style.fontWeight = "600";
    iconReview.style.background = "rgba(99,102,241,0.2)";
    iconReview.style.color = "#a5b4fc";
    iconReview.innerHTML = ICON_REVIEW;
    descReview.style.display = "block";

    // Export: upcoming
    stepExport.style.background = "transparent";
    stepExport.style.color = "#5a5a6e";
    stepExport.style.fontWeight = "400";
    iconExport.style.background = "rgba(255,255,255,0.04)";
    iconExport.style.color = "#4a4a5a";
    iconExport.innerHTML = ICON_EXPORT;
    descExport.style.display = "none";
  } else {
    // Review: completed
    stepReview.style.background = "transparent";
    stepReview.style.color = "#8b8ba0";
    stepReview.style.fontWeight = "400";
    iconReview.style.background = "rgba(34,197,94,0.15)";
    iconReview.style.color = "#4ade80";
    iconReview.innerHTML = ICON_CHECK;
    descReview.style.display = "none";

    // Export: active
    stepExport.style.background = "rgba(99,102,241,0.12)";
    stepExport.style.color = "#a5b4fc";
    stepExport.style.fontWeight = "600";
    iconExport.style.background = "rgba(99,102,241,0.2)";
    iconExport.style.color = "#a5b4fc";
    iconExport.innerHTML = ICON_EXPORT;
    descExport.style.display = "block";
  }

  // Update mobile tabs
  const mobileReview = document.getElementById("mock-mobile-review");
  const mobileExport = document.getElementById("mock-mobile-export");
  if (mobileReview && mobileExport) {
    mobileReview.style.color = tab === "review" ? "#a5b4fc" : "#5a5a6e";
    mobileReview.style.fontWeight = tab === "review" ? "600" : "400";
    mobileReview.style.borderBottomColor = tab === "review" ? "#818cf8" : "transparent";
    mobileExport.style.color = tab === "export" ? "#a5b4fc" : "#5a5a6e";
    mobileExport.style.fontWeight = tab === "export" ? "600" : "400";
    mobileExport.style.borderBottomColor = tab === "export" ? "#818cf8" : "transparent";
  }

  // Show/hide screens
  $("mock-review-screen").style.display = tab === "review" ? "flex" : "none";
  $("mock-export-screen").style.display = tab === "export" ? "flex" : "none";

  // Stop playback when switching
  if (isPlaying) togglePlay();
}

// ─── Video Playback ──────────────────────────────────────────

function togglePlay() {
  isPlaying = !isPlaying;
  const btn = $("mock-play-btn");

  if (isPlaying) {
    btn.innerHTML = `<svg width="18" height="18" viewBox="0 0 24 24" fill="white"><rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/></svg>`;
    playInterval = setInterval(() => {
      currentTime += 0.1;
      if (currentTime >= TOTAL_DURATION) {
        currentTime = 0;
        togglePlay();
      }
      updatePlaybackUI();
    }, 100);
  } else {
    btn.innerHTML = `<svg width="18" height="18" viewBox="0 0 24 24" fill="white"><polygon points="5,3 19,12 5,21"/></svg>`;
    if (playInterval) {
      clearInterval(playInterval);
      playInterval = null;
    }
  }
}

function seekTo(time: number) {
  currentTime = Math.max(0, Math.min(time, TOTAL_DURATION));
  activeSegmentIndex = getSegmentAtTime(currentTime);
  updatePlaybackUI();
  renderSegments();
}

function updatePlaybackUI() {
  const newIdx = getSegmentAtTime(currentTime);
  const segmentChanged = newIdx !== activeSegmentIndex;
  activeSegmentIndex = newIdx;

  // Timestamp
  $("mock-time-display").textContent =
    `${formatTime(currentTime)} / ${formatTime(TOTAL_DURATION)}`;

  // Scrubber progress
  const pct = (currentTime / TOTAL_DURATION) * 100;
  ($("mock-scrubber-fill") as HTMLElement).style.width = `${pct}%`;
  ($("mock-scrubber-head") as HTMLElement).style.left = `${pct}%`;

  // Timeline progress
  ($("mock-timeline-fill") as HTMLElement).style.width = `${pct}%`;
  ($("mock-timeline-head") as HTMLElement).style.left = `${pct}%`;

  // Caption
  const caption = $("mock-caption");
  const seg = SEGMENTS[activeSegmentIndex];
  caption.textContent = seg.text;
  caption.style.opacity = isPlaying ? "1" : "0";

  // Highlight thumbnails
  if (segmentChanged) {
    document.querySelectorAll(".mock-thumb").forEach((el, i) => {
      const thumb = el as HTMLElement;
      const overlay = thumb.querySelector(".mock-thumb-overlay") as HTMLElement | null;
      if (i === activeSegmentIndex) {
        thumb.style.border = "2px solid rgba(99,102,241,0.7)";
        if (overlay) overlay.style.backgroundColor = "rgba(99,102,241,0.3)";
      } else {
        thumb.style.border = "1px solid rgba(255,255,255,0.06)";
        if (overlay) overlay.style.backgroundColor = "rgba(0,0,0,0.45)";
      }
    });
    renderSegments();
  }
}

// ─── Segment Rendering ───────────────────────────────────────

function renderSegments() {
  const container = $("mock-segments");
  container.innerHTML = SEGMENTS.map((seg, i) => {
    const isCurrent = i === activeSegmentIndex;
    const rowBg = isCurrent ? "rgba(99,102,241,0.06)" : "transparent";
    const borderLeft = isCurrent
      ? "3px solid #6366f1"
      : "3px solid transparent";
    const tsColor = isCurrent ? "#818cf8" : "#5a5a6e";
    const textColor = isCurrent ? "#e0e0ea" : "#8b8ba0";
    const borderColor = isCurrent
      ? "rgba(99,102,241,0.3)"
      : "rgba(255,255,255,0.07)";

    return `
      <div class="mock-segment-row" data-index="${i}" style="display:grid;grid-template-columns:80px 1fr auto;gap:12px;align-items:start;padding:10px 14px;border-radius:6px;cursor:pointer;transition:all 0.12s;background:${rowBg};border-left:${borderLeft}">
        <div style="display:flex;flex-direction:column;padding-top:2px">
          <span style="font-family:ui-monospace,monospace;font-size:12px;font-weight:600;color:${tsColor}">${formatTime(seg.start_seconds)}</span>
          <span style="font-family:ui-monospace,monospace;font-size:10px;opacity:0.6;color:${tsColor}">${formatTime(seg.end_seconds)}</span>
        </div>
        <div style="display:flex;flex-direction:column;gap:4px">
          <textarea
            style="width:100%;resize:none;border-radius:6px;padding:8px 10px;font-size:13px;line-height:1.5;background:rgba(255,255,255,0.04);outline:none;border:1px solid ${borderColor};color:${textColor};font-family:inherit"
            rows="2"
            spellcheck="false"
          >${seg.text}</textarea>
          <span style="font-size:11px;font-style:italic;color:#5a5a6e">${seg.visual_description}</span>
        </div>
        <div style="display:flex;gap:4px;padding-top:2px">
          <span style="font-size:10px;padding:2px 6px;border-radius:4px;background:rgba(255,255,255,0.05);color:#5a5a6e">${seg.pace}</span>
          <button class="mock-segment-play" data-index="${i}" style="font-size:11px;color:#8b8ba0;background:none;border:none;cursor:pointer;font-family:inherit;padding:2px 6px;border-radius:4px">&#9654; Play</button>
          <button style="font-size:11px;color:#f87171;background:none;border:none;cursor:pointer;font-family:inherit;padding:2px 6px">Del</button>
        </div>
      </div>
    `;
  }).join("");

  container.style.display = "flex";
  container.style.flexDirection = "column";
  container.style.gap = "2px";

  // Click handlers
  container.querySelectorAll(".mock-segment-row").forEach((row) => {
    row.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).tagName === "TEXTAREA") return;
      const idx = parseInt((row as HTMLElement).dataset.index!);
      seekTo(SEGMENTS[idx].start_seconds);
    });
  });

  // Scroll active segment into view
  const activeRow = container.querySelector(
    `[data-index="${activeSegmentIndex}"]`
  );
  activeRow?.scrollIntoView({ behavior: "smooth", block: "nearest" });
}

// ─── Export Screen Interactivity ─────────────────────────────

const selectedFormats = new Set(["json", "srt"]);

function initExportScreen() {
  // Format toggle buttons
  document.querySelectorAll(".mock-format-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      const format = (btn as HTMLElement).dataset.format!;
      if (selectedFormats.has(format)) {
        selectedFormats.delete(format);
      } else {
        selectedFormats.add(format);
      }
      updateFormatButtons();
    });
  });

  // Section collapse toggles
  document.querySelectorAll(".mock-section-header").forEach((header) => {
    header.addEventListener("click", () => {
      const section = (header as HTMLElement).closest(".mock-section")!;
      const content = section.querySelector(".mock-section-content") as HTMLElement;
      const chevron = header.querySelector(".mock-chevron") as HTMLElement;

      if (content.style.display === "none") {
        content.style.display = "block";
        chevron.style.transform = "rotate(0deg)";
        (header as HTMLElement).style.borderBottom = "1px solid rgba(255,255,255,0.07)";
      } else {
        content.style.display = "none";
        chevron.style.transform = "rotate(-90deg)";
        (header as HTMLElement).style.borderBottom = "none";
      }
    });
  });

  // Toggle switches
  document.querySelectorAll(".mock-toggle").forEach((toggle) => {
    toggle.addEventListener("click", () => {
      const el = toggle as HTMLElement;
      const thumb = el.querySelector(".mock-toggle-thumb") as HTMLElement;
      const isOn = el.dataset.on === "true";
      el.dataset.on = isOn ? "false" : "true";
      el.style.backgroundColor = isOn
        ? "rgba(255,255,255,0.1)"
        : "#818cf8";
      thumb.style.left = isOn ? "2px" : "18px";
    });
  });

  // Radio buttons
  document.querySelectorAll(".mock-radio").forEach((radio) => {
    radio.addEventListener("click", () => {
      const group = (radio as HTMLElement).dataset.group!;
      document
        .querySelectorAll(`.mock-radio[data-group="${group}"]`)
        .forEach((r) => {
          const el = r as HTMLElement;
          const dot = el.querySelector(".mock-radio-dot") as HTMLElement;
          el.style.borderColor = "#5a5a6e";
          dot.style.opacity = "0";
        });
      const el = radio as HTMLElement;
      const dot = el.querySelector(".mock-radio-dot") as HTMLElement;
      el.style.borderColor = "#818cf8";
      dot.style.opacity = "1";
    });
  });

  // Mock export buttons
  document.querySelectorAll(".mock-export-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      const el = btn as HTMLElement;
      const originalText = el.textContent!;
      const bar = el
        .closest(".mock-section")
        ?.querySelector(".mock-progress-bar-fill") as HTMLElement | null;

      el.style.opacity = "0.6";
      el.style.pointerEvents = "none";
      el.textContent = "Exporting...";

      // Animate progress bar
      let progress = 0;
      const progressInterval = setInterval(() => {
        progress += Math.random() * 15 + 5;
        if (progress >= 100) {
          progress = 100;
          clearInterval(progressInterval);
          if (bar) bar.style.width = "100%";

          setTimeout(() => {
            el.textContent = originalText;
            el.style.opacity = "1";
            el.style.pointerEvents = "auto";
            if (bar) bar.style.width = "0%";

            // Show success message
            const successEl = el
              .closest(".mock-section-content")
              ?.querySelector(".mock-export-status") as HTMLElement | null;
            if (successEl) {
              successEl.style.display = "flex";
              setTimeout(() => {
                successEl.style.display = "none";
              }, 3000);
            }
          }, 500);
        }
        if (bar) bar.style.width = `${progress}%`;
      }, 150);
    });
  });

  updateFormatButtons();
}

function updateFormatButtons() {
  document.querySelectorAll(".mock-format-btn").forEach((btn) => {
    const format = (btn as HTMLElement).dataset.format!;
    const el = btn as HTMLElement;
    const active = selectedFormats.has(format);
    el.style.background = active ? "rgba(99,102,241,0.15)" : "transparent";
    el.style.borderColor = active ? "rgba(99,102,241,0.3)" : "rgba(255,255,255,0.07)";
    el.style.color = active ? "#818cf8" : "#5a5a6e";
    el.style.fontWeight = active ? "600" : "400";
  });
}

// ─── Scrubber Interactions ───────────────────────────────────

function initScrubbers() {
  // Video scrubber click
  const scrubber = $("mock-scrubber");
  scrubber.addEventListener("click", (e) => {
    const rect = scrubber.getBoundingClientRect();
    const pct = (e.clientX - rect.left) / rect.width;
    seekTo(pct * TOTAL_DURATION);
  });

  // Timeline click
  const timeline = $("mock-timeline");
  timeline.addEventListener("click", (e) => {
    const rect = timeline.getBoundingClientRect();
    const pct = (e.clientX - rect.left) / rect.width;
    seekTo(pct * TOTAL_DURATION);
  });

  // Play button
  $("mock-play-btn").addEventListener("click", togglePlay);

  // Video area click to play/pause
  $("mock-video-area").addEventListener("click", togglePlay);
}

// ─── Init ────────────────────────────────────────────────────

export function initMockApp() {
  // Tab switching (sidebar + mobile)
  $("mock-step-review").addEventListener("click", () => switchTab("review"));
  $("mock-step-export").addEventListener("click", () => switchTab("export"));
  document.getElementById("mock-mobile-review")?.addEventListener("click", () => switchTab("review"));
  document.getElementById("mock-mobile-export")?.addEventListener("click", () => switchTab("export"));

  initScrubbers();
  renderSegments();
  initExportScreen();
  updatePlaybackUI();
}
