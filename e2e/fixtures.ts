import { test as base, expect, type Page } from "@playwright/test";

const PROOF = "e2e-proof";

type Users = {
  /** Signed in as max, the way Realm would sign him in. */
  max: Page;
  anna: Page;
  /** No identity headers: what the public domain sees. */
  anon: Page;
  /** Every CSP violation reported on any of the pages above. */
  cspViolations: string[];
};

export const test = base.extend<Users>({
  cspViolations: async ({}, use) => {
    await use([]);
  },
  max: async ({ browser, contextOptions, viewport, isMobile, hasTouch, userAgent, baseURL, cspViolations }, use) => {
    await use(await pageFor({ browser, contextOptions, viewport, isMobile, hasTouch, userAgent, baseURL }, "max", cspViolations));
  },
  anna: async ({ browser, contextOptions, viewport, isMobile, hasTouch, userAgent, baseURL, cspViolations }, use) => {
    await use(await pageFor({ browser, contextOptions, viewport, isMobile, hasTouch, userAgent, baseURL }, "anna", cspViolations));
  },
  anon: async ({ browser, contextOptions, viewport, isMobile, hasTouch, userAgent, baseURL, cspViolations }, use) => {
    await use(await pageFor({ browser, contextOptions, viewport, isMobile, hasTouch, userAgent, baseURL }, null, cspViolations));
  },
});

test.afterEach(async ({ cspViolations }) => {
  expect(cspViolations, "Content-Security-Policy violations").toEqual([]);
});

async function pageFor(o: any, user: string | null, violations: string[]): Promise<Page> {
  const context = await o.browser.newContext({
    ...o.contextOptions,
    viewport: o.viewport,
    isMobile: o.isMobile,
    hasTouch: o.hasTouch,
    userAgent: o.userAgent,
    baseURL: o.baseURL,
    extraHTTPHeaders: user ? { "x-kvitter-user": user, "x-kvitter-proof": PROOF } : {},
  });
  const page = await context.newPage();
  page.on("console", (msg) => {
    if (/Content Security Policy/i.test(msg.text())) violations.push(`${user ?? "anon"}: ${msg.text()}`);
  });
  return page;
}

/** A word no other test run has used, for finding our own posts. */
export function unique(prefix = "w"): string {
  return `${prefix}${Date.now().toString(36)}${Math.floor(Math.random() * 1e4)}`;
}

/** Writes a post through the CodeMirror composer and returns its id. */
export async function compose(page: Page, text: string, visibility: "private" | "public" = "private"): Promise<string> {
  await page.goto("/");
  const editor = page.locator(".cm-content");
  await editor.click();
  await page.keyboard.insertText(text);
  if (visibility === "public") await page.locator("form.composer label.pub").click();
  await page.getByRole("button", { name: visibility === "public" ? "Post publicly" : "Post privately" }).click();
  await expect(page).toHaveURL("/");
  // The feed is newest first, and tests run one at a time.
  const href = await page.locator("article.post a.when").first().getAttribute("href");
  return href!.replace("/p/", "");
}

export { expect };
