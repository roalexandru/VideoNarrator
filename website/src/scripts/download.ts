const REPO = "roalexandru/VideoNarrator";
const RELEASES_URL = `https://github.com/${REPO}/releases`;
const API_URL = `https://api.github.com/repos/${REPO}/releases/latest`;

interface ReleaseAsset {
  name: string;
  browser_download_url: string;
  size: number;
}

interface Release {
  tag_name: string;
  assets: ReleaseAsset[];
  html_url: string;
}

type Platform = "mac-arm" | "mac-intel" | "windows" | "unknown";

function detectPlatform(): Platform {
  const ua = navigator.userAgent.toLowerCase();
  const platform = (navigator as { userAgentData?: { platform?: string } })
    .userAgentData?.platform?.toLowerCase() ?? navigator.platform?.toLowerCase() ?? "";

  if (platform.includes("mac") || ua.includes("macintosh")) {
    // Check for Apple Silicon — userAgentData doesn't expose arch reliably,
    // so default to ARM as all Macs since late 2020 are Apple Silicon
    return "mac-arm";
  }
  if (platform.includes("win") || ua.includes("windows")) {
    return "windows";
  }
  return "unknown";
}

function getPlatformLabel(platform: Platform): string {
  switch (platform) {
    case "mac-arm": return "macOS (Apple Silicon)";
    case "mac-intel": return "macOS (Intel)";
    case "windows": return "Windows";
    default: return "your platform";
  }
}

function getPlatformIcon(platform: Platform): string {
  if (platform === "windows") {
    return `<svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor"><path d="M0 3.449L9.75 2.1v9.451H0m10.949-9.602L24 0v11.4H10.949M0 12.6h9.75v9.451L0 20.699M10.949 12.6H24V24l-12.9-1.801"/></svg>`;
  }
  // Apple icon for mac
  return `<svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor"><path d="M18.71 19.5c-.83 1.24-1.71 2.45-3.05 2.47-1.34.03-1.77-.79-3.29-.79-1.53 0-2 .77-3.27.82-1.31.05-2.3-1.32-3.14-2.53C4.25 17 2.94 12.45 4.7 9.39c.87-1.52 2.43-2.48 4.12-2.51 1.28-.02 2.5.87 3.29.87.78 0 2.26-1.07 3.8-.91.65.03 2.47.26 3.64 1.98-.09.06-2.17 1.28-2.15 3.81.03 3.02 2.65 4.03 2.68 4.04-.03.07-.42 1.44-1.38 2.83M13 3.5c.73-.83 1.94-1.46 2.94-1.5.13 1.17-.34 2.35-1.04 3.19-.69.85-1.83 1.51-2.95 1.42-.15-1.15.41-2.35 1.05-3.11z"/></svg>`;
}

function findAsset(assets: ReleaseAsset[], platform: Platform): ReleaseAsset | null {
  for (const asset of assets) {
    const name = asset.name.toLowerCase();
    switch (platform) {
      case "mac-arm":
        if (name.includes("aarch64") && (name.endsWith(".dmg") || name.endsWith(".app.tar.gz"))) {
          return name.endsWith(".dmg") ? asset : null;
        }
        break;
      case "mac-intel":
        if (name.includes("x86_64") && name.includes("darwin") && (name.endsWith(".dmg") || name.endsWith(".app.tar.gz"))) {
          return name.endsWith(".dmg") ? asset : null;
        }
        break;
      case "windows":
        if (name.endsWith(".msi") || (name.endsWith(".exe") && name.includes("setup"))) {
          return asset;
        }
        break;
    }
  }
  // Fallback: try .dmg for mac, .msi for windows
  for (const asset of assets) {
    const name = asset.name.toLowerCase();
    if (platform === "mac-arm" && name.includes("aarch64") && (name.endsWith(".dmg") || name.endsWith(".tar.gz"))) return asset;
    if (platform === "mac-intel" && name.includes("x86_64") && name.includes("darwin")) return asset;
    if (platform === "windows" && (name.endsWith(".msi") || name.endsWith(".exe"))) return asset;
  }
  return null;
}

function formatSize(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  return `${mb.toFixed(1)} MB`;
}

function renderDownloads(release: Release | null) {
  const platform = detectPlatform();
  const version = release?.tag_name ?? "";
  const versionLabel = version ? ` ${version}` : "";

  const primaryBtn = document.getElementById("dl-primary-btn") as HTMLAnchorElement | null;
  const primaryLabel = document.getElementById("dl-primary-label");
  const primaryMeta = document.getElementById("dl-primary-meta");
  const versionEl = document.getElementById("dl-version");
  const altContainer = document.getElementById("dl-alternatives");

  if (versionEl && version) {
    versionEl.textContent = `Latest: ${version}`;
    versionEl.style.display = "block";
  }

  if (release && platform !== "unknown") {
    const asset = findAsset(release.assets, platform);
    if (asset && primaryBtn && primaryLabel && primaryMeta) {
      primaryBtn.href = asset.browser_download_url;
      primaryLabel.innerHTML = `${getPlatformIcon(platform)} Download for ${getPlatformLabel(platform)}`;
      primaryMeta.textContent = `${formatSize(asset.size)}${versionLabel}`;
      primaryMeta.style.display = "block";
    }
  }

  // Render alternative downloads
  if (altContainer && release) {
    const alternatives: { label: string; platform: Platform; icon: string }[] = [];

    if (platform !== "mac-arm") alternatives.push({ label: "macOS (Apple Silicon)", platform: "mac-arm", icon: getPlatformIcon("mac-arm") });
    if (platform !== "mac-intel") alternatives.push({ label: "macOS (Intel)", platform: "mac-intel", icon: getPlatformIcon("mac-intel") });
    if (platform !== "windows") alternatives.push({ label: "Windows", platform: "windows", icon: getPlatformIcon("windows") });

    const links = alternatives
      .map((alt) => {
        const asset = findAsset(release.assets, alt.platform);
        if (!asset) return "";
        return `<a href="${asset.browser_download_url}" class="inline-flex items-center gap-2 rounded-lg border border-border bg-surface-card px-4 py-2.5 text-sm text-text-muted transition-colors hover:border-text-muted hover:text-text-primary">
          ${alt.icon}
          ${alt.label}
          <span class="text-xs opacity-60">${formatSize(asset.size)}</span>
        </a>`;
      })
      .filter(Boolean);

    if (links.length > 0) {
      altContainer.innerHTML = `
        <p class="mb-3 text-sm text-text-muted">Also available for:</p>
        <div class="flex flex-wrap justify-center gap-3">${links.join("")}</div>
      `;
      altContainer.style.display = "block";
    }
  }
}

export function initDownloads() {
  // Try to fetch latest release info from GitHub API
  fetch(API_URL)
    .then((res) => {
      if (!res.ok) throw new Error("No release");
      return res.json() as Promise<Release>;
    })
    .then((release) => renderDownloads(release))
    .catch(() => renderDownloads(null));
}
