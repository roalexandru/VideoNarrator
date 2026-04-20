export const APP_VERSION = __APP_VERSION__;
export const APP_BUILD_TIME = __APP_BUILD_TIME__;
export const APP_GIT_SHA = __APP_GIT_SHA__;
export const IS_DEV = import.meta.env.DEV;
export const DISPLAY_VERSION = IS_DEV ? `${APP_VERSION}-dev` : APP_VERSION;

export function formatBuildTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
