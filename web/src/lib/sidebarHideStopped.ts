import { safeGetItem, safeSetItem } from "./safeStorage";

/** Stopped sessions inside groups: all shown, their rows hidden, or their rows and any group left empty hidden. Per-browser, like the axis. */
export type SidebarHideStopped = "off" | "rows" | "groups";

export const SIDEBAR_HIDE_STOPPED_KEY = "aoe-sidebar-hide-stopped";

export const NEXT_HIDE_STOPPED: Record<SidebarHideStopped, SidebarHideStopped> = {
  off: "rows",
  rows: "groups",
  groups: "off",
};

export const HIDE_STOPPED_LABEL: Record<SidebarHideStopped, string> = {
  off: "Stopped sessions shown",
  rows: "Stopped sessions hidden in groups",
  groups: "Stopped sessions and their emptied groups hidden",
};

export function loadSidebarHideStopped(): SidebarHideStopped {
  const value = safeGetItem(SIDEBAR_HIDE_STOPPED_KEY);
  return value === "rows" || value === "groups" ? value : "off";
}

export function saveSidebarHideStopped(mode: SidebarHideStopped): void {
  safeSetItem(SIDEBAR_HIDE_STOPPED_KEY, mode);
}
