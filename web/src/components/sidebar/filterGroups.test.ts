// @vitest-environment node

import { describe, expect, it } from "vitest";

import {
  buildNestedSidebarGroups,
  buildOrgGroups,
  buildSessionGroups,
  nestedSidebarGroupShouldRender,
  repoGroupToSidebarGroup,
  sidebarGroupShouldRender,
  UNGROUPED_GROUP_ID,
  type SidebarGroup,
} from "../../lib/sidebarGroups";
import type { RepoGroup, SessionResponse, SessionStatus, Workspace } from "../../lib/types";
import { makeSession, makeWorkspace } from "../__tests__/fixtures";
import { hideStoppedFlat, hideStoppedNested, hideStoppedOrg } from "./filterGroups";

const ARCHIVED = { archived_at: "2026-01-01T00:00:00Z" };

const ws = (id: string, group: string, statuses: SessionStatus[], over: Partial<SessionResponse> = {}): Workspace =>
  makeWorkspace(
    id,
    statuses.map((status, i) => makeSession({ id: `${id}-s${i}`, group_path: group, status, ...over })),
  );

const byGroup = (workspaces: Workspace[]) =>
  buildSessionGroups(workspaces, { idleDecayWindowMs: 60_000, sortMode: "manual", isCollapsed: () => false });

function repoGroup(id: string, workspaces: Workspace[], remoteOwner: string | null = null): RepoGroup {
  return {
    id,
    repoPath: id,
    displayName: id,
    defaultDisplayName: id,
    alias: null,
    color: null,
    remoteOwner,
    remoteOwnerKey: remoteOwner,
    workspaces,
    status: "idle",
    collapsed: false,
    registeredProjects: [],
  } as RepoGroup;
}

const rowIds = (g: SidebarGroup) => g.workspaces.map((v) => v.workspace.id);
const find = (groups: SidebarGroup[], id: string) => groups.find((g) => g.id === id)!;

describe("hideStoppedFlat on the group axis", () => {
  const groups = byGroup([
    ws("stopped", "feature", ["Stopped"]),
    ws("running", "feature", ["Running"]),
    ws("mixed", "feature", ["Stopped", "Idle"]),
    ws("archived", "feature", ["Stopped"], ARCHIVED),
    ws("only-stopped", "refactor", ["Stopped"]),
    ws("loose", "", ["Stopped"]),
  ]);

  it("off changes nothing", () => {
    expect(hideStoppedFlat(groups, "off", "stopped")).toBe(groups);
  });

  it("rows hides grouped stopped rows, keeps the rest, and keeps every header", () => {
    const shown = hideStoppedFlat(groups, "rows", null);
    expect(rowIds(find(shown, "feature"))).toEqual(rowIds(find(groups, "feature")).filter((id) => id !== "stopped"));
    expect(find(shown, "feature").hidden).toEqual({ ids: ["stopped"], keepHeader: true });
    expect(rowIds(find(shown, "refactor"))).toEqual([]);
    expect(find(shown, "refactor").hidden).toEqual({ ids: ["only-stopped"], keepHeader: true });
    expect(find(shown, UNGROUPED_GROUP_ID)).toBe(find(groups, UNGROUPED_GROUP_ID));
    expect(shown.filter(sidebarGroupShouldRender).map((g) => g.id)).toEqual(groups.map((g) => g.id));
  });

  it("groups also drops a group it emptied, unless it holds the open workspace", () => {
    const shown = hideStoppedFlat(groups, "groups", null);
    expect(shown.filter(sidebarGroupShouldRender).map((g) => g.id)).toEqual(["feature", UNGROUPED_GROUP_ID]);
    const withOpen = hideStoppedFlat(groups, "groups", "only-stopped");
    expect(withOpen.filter(sidebarGroupShouldRender).map((g) => g.id)).toEqual(groups.map((g) => g.id));
  });

  it("leaves an emptied group's sunk rows for the footer without rendering its header", () => {
    const sunkOnly = byGroup([ws("s", "g", ["Stopped"]), ws("a", "g", ["Idle"], ARCHIVED)]);
    const [g] = hideStoppedFlat(sunkOnly, "groups", null);
    expect(rowIds(g!)).toEqual(["a"]);
    expect(sidebarGroupShouldRender(g!)).toBe(false);
  });
});

describe("hiding on the repo, repo+group and org axes", () => {
  const repos = [
    repoGroup("/repo-a", [ws("a-stopped", "", ["Stopped"]), ws("a-idle", "feature", ["Idle"])], "acme"),
    repoGroup("/repo-b", [ws("b-stopped", "feature", ["Stopped"])], "acme"),
  ];

  it("repo: no group is exempt, so ungrouped sessions hide too", () => {
    const shown = hideStoppedFlat(repos.map(repoGroupToSidebarGroup), "rows", null);
    expect(shown.map(rowIds)).toEqual([["a-idle"], []]);
    expect(shown.map((g) => g.hidden?.ids)).toEqual([["a-stopped"], ["b-stopped"]]);
  });

  it("a pinned project keeps its header after hiding emptied it, on the repo and repo+group axes", () => {
    const pin = { name: "b", path: "/repo-b", scope: "global" as const, pinned: true };
    const both = [{ ...repos[1]!, registeredProjects: [pin] }, repos[1]!];
    expect(hideStoppedFlat(both.map(repoGroupToSidebarGroup), "groups", null).map(sidebarGroupShouldRender)).toEqual([
      true,
      false,
    ]);
    const nested = buildNestedSidebarGroups(both, {
      idleDecayWindowMs: 60_000,
      sortMode: "manual",
      isSubgroupCollapsed: () => false,
    });
    expect(hideStoppedNested(nested, "groups", null).map(nestedSidebarGroupShouldRender)).toEqual([true, false]);
  });

  it("repo+group: a repo's Ungrouped subgroup hides too, and the repo header counts what it lost", () => {
    const nested = buildNestedSidebarGroups(repos, {
      idleDecayWindowMs: 60_000,
      sortMode: "manual",
      isSubgroupCollapsed: () => false,
    });
    const [a, b] = hideStoppedNested(nested, "rows", null);
    expect(a!.repo.hidden?.ids).toEqual(["a-stopped"]);
    expect(find(a!.subgroups, UNGROUPED_GROUP_ID).hidden?.ids).toEqual(["a-stopped"]);
    expect(rowIds(find(a!.subgroups, "feature"))).toEqual(["a-idle"]);
    expect(b!.subgroups.map(sidebarGroupShouldRender)).toEqual([true]);
    expect(hideStoppedNested(nested, "groups", null)[1]!.subgroups.map(sidebarGroupShouldRender)).toEqual([false]);
  });

  it("org: repos and the org header both hide their stopped rows", () => {
    const orgs = buildOrgGroups(repos, { isOrgCollapsed: () => false, isRepoCollapsed: () => false });
    const [acme] = hideStoppedOrg(orgs, "groups", null);
    expect(acme!.org.hidden?.ids).toEqual(["a-stopped", "b-stopped"]);
    expect(rowIds(acme!.org)).toEqual(["a-idle"]);
    expect(acme!.repos.map(sidebarGroupShouldRender)).toEqual([true, false]);
  });
});
