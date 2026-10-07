// The `y` filter: stopped sessions inside groups hide, then so do the groups they emptied. The bucketing
// rules are unit-tested in src/components/sidebar/filterGroups.test.ts.

import { test, expect } from "./helpers/mockedTest";
import type { Page } from "@playwright/test";
import { installSidebarMocks, type MockSessionInput } from "./helpers/sidebarMocks";

const HEADER = "[data-testid='sidebar-group-header']";
const ROW = "[data-testid='sidebar-session-row']";
const TOGGLE = "[data-testid='sidebar-hide-stopped-toggle']";
const COUNT = "[data-testid='sidebar-group-session-count']";
const STOPPED = { status: "Stopped" };

function sessions(): MockSessionInput[] {
  return [
    { id: "s-f1", title: "feat-one", project_path: "/tmp/p", branch: "feat/one", group: "feature", fields: STOPPED },
    { id: "s-f2", title: "feat-two", project_path: "/tmp/p", branch: "feat/two", group: "feature" },
    { id: "s-r1", title: "refac-one", project_path: "/tmp/p", branch: "refac/one", group: "refactor", fields: STOPPED },
    { id: "s-l1", title: "loose-one", project_path: "/tmp/p", branch: "loose/one", fields: STOPPED },
  ];
}

async function openOnGroupAxis(page: Page, path = "/", hideStopped?: string) {
  await page.addInitScript((mode) => {
    localStorage.setItem("aoe-sidebar-axis", "group");
    if (mode && !sessionStorage.getItem("seeded")) {
      localStorage.setItem("aoe-sidebar-hide-stopped", mode);
      sessionStorage.setItem("seeded", "1");
    }
  }, hideStopped);
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto(path);
}

const header = (page: Page, name: string) => page.locator(HEADER, { has: page.getByText(name, { exact: true }) });

test.describe("sidebar hide stopped sessions in groups", () => {
  test("y cycles hiding stopped rows, then the groups they emptied, then back; a poll hides live", async ({ page }) => {
    const mocks = await installSidebarMocks(page, { sessions: sessions() });
    await openOnGroupAxis(page);
    const rows = page.locator(ROW);
    const toggle = page.locator(TOGGLE);
    await expect(rows).toHaveCount(4);
    await expect(toggle).toHaveAttribute("data-mode", "off");

    await page.keyboard.press("y");
    await expect(toggle).toHaveAttribute("data-mode", "rows");
    await expect(toggle).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByText("Stopped sessions hidden in groups").last()).toBeVisible();
    await expect(rows).toHaveCount(2);
    await expect(page.getByText("feat-two")).toBeVisible();
    await expect(page.getByText("loose-one")).toBeVisible();
    await expect(page.getByText("feat-one")).toBeHidden();
    await expect(header(page, "feature").locator(COUNT)).toHaveText("(1/2)");
    await expect(header(page, "refactor").locator(COUNT)).toHaveText("(0/1)");
    await expect(header(page, "Ungrouped").locator(COUNT)).toHaveText("(1)");

    mocks.patchSession("s-f2", STOPPED);
    await expect(page.getByText("feat-two")).toBeHidden({ timeout: 10_000 });
    await expect(header(page, "feature").locator(COUNT)).toHaveText("(0/2)");

    await page.keyboard.press("y");
    await expect(toggle).toHaveAttribute("data-mode", "groups");
    await expect(header(page, "feature")).toHaveCount(0);
    await expect(header(page, "refactor")).toHaveCount(0);
    await expect(header(page, "Ungrouped")).toHaveCount(1);
    await expect(rows).toHaveCount(1);

    await page.reload();
    await expect(toggle).toHaveAttribute("data-mode", "groups");
    await expect(rows).toHaveCount(1);

    await toggle.click();
    await expect(toggle).toHaveAttribute("data-mode", "off");
    await expect(toggle).toHaveAttribute("aria-pressed", "false");
    await expect(rows).toHaveCount(4);
    await expect(page.locator(HEADER)).toHaveCount(3);
  });

  test("the open session's hidden row stays open and marks its group header", async ({ page }) => {
    await installSidebarMocks(page, { sessions: sessions() });
    await openOnGroupAxis(page, "/session/s-r1", "rows");
    const refactor = header(page, "refactor");
    await expect(page.locator(TOGGLE)).toHaveAttribute("data-mode", "rows");
    await expect(page.locator(ROW, { hasText: "refac-one" })).toHaveCount(0);
    await expect(refactor).toHaveClass(/border-session-active/);
    await expect(header(page, "feature")).not.toHaveClass(/border-session-active/);

    await page.keyboard.press("y");
    await expect(page.locator(TOGGLE)).toHaveAttribute("data-mode", "groups");
    await expect(refactor).toHaveClass(/border-session-active/);
    await expect(header(page, "feature")).toHaveCount(1);
    expect(new URL(page.url()).pathname).toBe("/session/s-r1");

    await page.keyboard.press("y");
    await expect(page.locator(ROW, { hasText: "refac-one" })).toHaveCount(1);
    await expect(refactor).not.toHaveClass(/border-session-active/);
  });
});
