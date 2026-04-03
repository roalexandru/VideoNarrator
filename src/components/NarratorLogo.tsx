interface LogoProps {
  size?: number;
  showText?: boolean;
}

export function NarratorLogo({ size = 30, showText = true }: LogoProps) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
      <svg width={size} height={size} viewBox="0 0 64 64" fill="none" xmlns="http://www.w3.org/2000/svg">
        <defs>
          <linearGradient id="logo-grad" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stopColor="#6366f1" />
            <stop offset="100%" stopColor="#a855f7" />
          </linearGradient>
          <linearGradient id="play-grad" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stopColor="#fff" stopOpacity="0.95" />
            <stop offset="100%" stopColor="#e0e7ff" stopOpacity="0.9" />
          </linearGradient>
        </defs>
        {/* Rounded square background */}
        <rect x="2" y="2" width="60" height="60" rx="14" fill="url(#logo-grad)" />
        {/* Subtle film strip lines */}
        <rect x="8" y="8" width="4" height="4" rx="1" fill="rgba(255,255,255,0.15)" />
        <rect x="8" y="16" width="4" height="4" rx="1" fill="rgba(255,255,255,0.15)" />
        <rect x="8" y="24" width="4" height="4" rx="1" fill="rgba(255,255,255,0.1)" />
        <rect x="52" y="36" width="4" height="4" rx="1" fill="rgba(255,255,255,0.1)" />
        <rect x="52" y="44" width="4" height="4" rx="1" fill="rgba(255,255,255,0.15)" />
        <rect x="52" y="52" width="4" height="4" rx="1" fill="rgba(255,255,255,0.15)" />
        {/* Play triangle */}
        <path d="M24 18 L48 32 L24 46 Z" fill="url(#play-grad)" />
        {/* Sound wave lines */}
        <line x1="36" y1="50" x2="36" y2="56" stroke="rgba(255,255,255,0.4)" strokeWidth="2" strokeLinecap="round" />
        <line x1="40" y1="48" x2="40" y2="56" stroke="rgba(255,255,255,0.3)" strokeWidth="2" strokeLinecap="round" />
        <line x1="44" y1="50" x2="44" y2="56" stroke="rgba(255,255,255,0.2)" strokeWidth="2" strokeLinecap="round" />
      </svg>
      {showText && (
        <span style={{ fontWeight: 700, fontSize: size * 0.53, color: "#e0e0ea", letterSpacing: -0.3 }}>
          Narrator
        </span>
      )}
    </div>
  );
}
