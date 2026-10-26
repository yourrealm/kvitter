import { readFileSync } from "node:fs";
import { compose, expect, test, unique } from "./fixtures";

const PUBLIC_URL = "https://kvitter.example";

test("a new post is private, styled in place, and visible only to its author", async ({ max, anna, anon, isMobile }) => {
  const word = unique();
  await max.goto("/");
  await max.locator(".cm-content").click();
  await max.keyboard.insertText(`**bold** ${word} #notes`);

  // Markdown stays verbatim: the ** are there, dimmed, and the tag is highlighted.
  await expect(max.locator(".cm-content")).toContainText(`**bold** ${word} #notes`);
  await expect(max.locator(".cm-md").first()).toHaveText("**");
  await expect(max.locator(".cm-md-tag")).toHaveText("#notes");

  // Private is the default, and the button says so.
  await expect(max.getByRole("button", { name: "Post privately" })).toBeVisible();
  // Phones have no Ctrl+Enter (and emulation mixes up Ctrl and Cmd), so they tap the button.
  if (isMobile) await max.getByRole("button", { name: "Post privately" }).click();
  else await max.keyboard.press("ControlOrMeta+Enter");
  await expect(max).toHaveURL("/");

  const post = max.locator("article.post").first();
  await expect(post).toContainText(word);
  await expect(post.locator(".vis")).toHaveText("Private");
  await expect(post.locator("strong")).toHaveText("bold");
  const id = (await post.locator("a.when").getAttribute("href"))!.replace("/p/", "");

  for (const page of [anna, anon]) {
    const res = await page.goto(`/p/${id}`);
    expect(res!.status()).toBe(404);
    await expect(page.locator("body")).not.toContainText(word);
  }
  await anna.goto("/explore");
  await expect(anna.locator("main")).not.toContainText(word);
});

test("making a post public shows it on explore and offers the public URL", async ({ max, anna, anon }) => {
  const word = unique();
  const id = await compose(max, `Sauna at the lake ${word} #sauna`);

  await max.goto(`/p/${id}`);
  await expect(max.getByRole("button", { name: "Copy public URL" })).toHaveCount(0);
  await max.getByRole("button", { name: "Public" }).click();
  await expect(max.locator(".single .vis")).toHaveText("Public");
  await expect(max.getByRole("button", { name: "Copy public URL" })).toBeVisible();
  await expect(max.locator("[data-copy-text]")).toHaveText(`${PUBLIC_URL}/p/${id}`);

  await anna.goto("/explore");
  await expect(anna.locator("article.post", { hasText: word }).locator(".author")).toHaveText("max");

  await anon.goto(`/p/${id}`);
  await expect(anon.locator(".single")).toContainText(word);
  // Tag pages don't exist on the public domain, so tags aren't links there.
  await expect(anon.locator(".single .tag")).toHaveText("#sauna");
  await expect(anon.locator(".single a.tag")).toHaveCount(0);
  await expect(anon.locator('link[rel="canonical"]')).toHaveAttribute("href", `${PUBLIC_URL}/p/${id}`);
  await expect(anon.locator("form")).toHaveCount(0);

  const rss = await anon.request.get("/rss.xml");
  expect(await rss.text()).toContain(`${PUBLIC_URL}/p/${id}`);

  // And back to private: gone for everyone else.
  await max.getByRole("button", { name: "Private" }).click();
  await expect(max.locator(".single .vis")).toHaveText("Private");
  // Not anon.goto: its browser may still hold the page it was allowed to cache for a minute.
  expect((await anon.request.get(`/p/${id}`)).status()).toBe(404);
  expect(await (await anon.request.get("/rss.xml")).text()).not.toContain(id);
});

test("tags autocomplete from posts you can see", async ({ max }) => {
  const tag = unique("tag");
  await compose(max, `first #${tag}`);
  await max.goto("/");
  await max.locator(".cm-content").click();
  await max.keyboard.type(`more #${tag.slice(0, 6)}`);
  const option = max.locator(".cm-tooltip-autocomplete li", { hasText: `#${tag}` });
  await option.click();
  await expect(max.locator(".cm-content")).toContainText(`more #${tag}`);
});

test("images upload stripped and resized, and follow the post's visibility", async ({ max, anon }) => {
  await max.goto("/");
  await expect(max.getByRole("button", { name: "Image" })).toBeVisible();
  await max.locator("[data-upload-input]").setInputFiles({
    name: "photo.png",
    mimeType: "image/png",
    buffer: readFileSync("e2e/photo.png"),
  });
  await expect(max.locator(".cm-content")).toContainText(/!\[\]\(\/files\/[a-z2-9]{8}\)/);
  const text = await max.locator(".cm-content").innerText();
  const fileId = text.match(/\/files\/([a-z2-9]{8})/)![1];

  await max.locator(".cm-content").click();
  await max.keyboard.insertText(" lake photo");
  await max.locator("form.composer label.pub").click();
  await max.getByRole("button", { name: "Post publicly" }).click();

  const img = max.locator("article.post").first().locator("img");
  await expect(img).toHaveAttribute("src", `/files/${fileId}`);
  await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBeGreaterThan(0);

  const anonFile = await anon.request.get(`/files/${fileId}`);
  expect(anonFile.status()).toBe(200);
  expect(anonFile.headers()["content-type"]).toBe("image/jpeg");

  const postId = (await max.locator("article.post a.when").first().getAttribute("href"))!.replace("/p/", "");
  await max.goto(`/p/${postId}`);
  await max.getByRole("button", { name: "Private" }).click();
  await expect(max.locator(".single .vis")).toHaveText("Private");
  expect((await anon.request.get(`/files/${fileId}`)).status()).toBe(404);
});

test("edit and delete", async ({ max }) => {
  const word = unique();
  const id = await compose(max, `draft ${word}`);
  await max.goto(`/p/${id}`);
  await max.getByRole("link", { name: "Edit" }).click();
  await expect(max).toHaveURL(`/p/${id}/edit`);
  await expect(max.locator(".cm-content")).toContainText(`draft ${word}`);
  await max.locator(".cm-content").click();
  await max.keyboard.press("ControlOrMeta+End");
  await max.keyboard.insertText(" (edited)");
  await max.getByRole("button", { name: "Save" }).click();
  await expect(max).toHaveURL(`/p/${id}`);
  await expect(max.locator(".single .body")).toContainText(`draft ${word} (edited)`);

  await max.getByRole("link", { name: "Edit" }).click();
  await max.getByText("Delete post").click();
  await max.getByRole("button", { name: "Delete permanently" }).click();
  await expect(max).toHaveURL("/");
  await expect(max.locator("main")).not.toContainText(word);
  expect((await max.goto(`/p/${id}`))!.status()).toBe(404);
});

test("validation keeps the draft", async ({ max }) => {
  await max.goto("/");
  await max.getByRole("button", { name: "Post privately" }).click();
  await expect(max.getByRole("alert")).toHaveText("Write something first.");
});

test("anonymous visitors get the read-only site", async ({ anon }) => {
  await anon.goto("/");
  await expect(anon).toHaveURL("/explore");
  await expect(anon.locator("form")).toHaveCount(0);
  await expect(anon.locator("script")).toHaveCount(0);
  for (const path of ["/u/max", "/t/sauna", "/api/tags?q="]) {
    expect((await anon.request.get(path)).status(), path).toBe(404);
  }
  const robots = await (await anon.request.get("/robots.txt")).text();
  expect(robots).toContain(`Sitemap: ${PUBLIC_URL}/sitemap.xml`);
});

test("the composer works without JavaScript", async ({ browser, baseURL }) => {
  const context = await browser.newContext({
    baseURL,
    javaScriptEnabled: false,
    extraHTTPHeaders: { "x-kvitter-user": "max", "x-kvitter-proof": "e2e-proof" },
  });
  const page = await context.newPage();
  const word = unique();
  await page.goto("/");
  await page.locator("textarea[name=body]").fill(`plain ${word}`);
  await page.getByRole("button", { name: "Post privately" }).click();
  await expect(page.locator("article.post").first()).toContainText(word);
  await context.close();
});

test("no horizontal scrolling", async ({ max, anon }) => {
  const id = await compose(max, `${"https://example.com/a-very-long-url-without-any-breaks-".repeat(3)} ${unique()}`, "public");
  for (const [page, path] of [[max, "/"], [max, `/p/${id}`], [anon, "/explore"], [anon, `/p/${id}`]] as const) {
    await page.goto(path);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, path).toBeLessThanOrEqual(0);
  }
});
