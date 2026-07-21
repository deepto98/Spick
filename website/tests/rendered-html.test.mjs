import assert from "node:assert/strict";
import { access } from "node:fs/promises";
import test from "node:test";

async function render() {
  const workerUrl = new URL("../dist/server/index.js", import.meta.url);
  workerUrl.searchParams.set("test", `${process.pid}-${Date.now()}`);
  const { default: worker } = await import(workerUrl.href);

  return worker.fetch(
    new Request("http://localhost/", {
      headers: { accept: "text/html" },
    }),
    {
      ASSETS: {
        fetch: async () => new Response("Not found", { status: 404 }),
      },
    },
    {
      waitUntil() {},
      passThroughOnException() {},
    },
  );
}

test("server-renders the complete Spick landing page", async () => {
  const response = await render();
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type") ?? "", /^text\/html\b/i);

  const html = await response.text();
  assert.match(html, /<title>Spick — Voice typing for macOS<\/title>/i);
  assert.match(html, /Keep your hands on the work\./);
  assert.match(html, /Works where your cursor works/);
  assert.match(html, /Your Mac can do the listening\./);
  assert.match(html, /Notes made for speaking/);
  assert.match(html, />Download for Mac</);
  assert.match(
    html,
    /href=["']https:\/\/github\.com\/deepto98\/Spick\/releases\/download\/v0\.1\.0-preview\.1\/Spick_0\.1\.0_local_aarch64\.dmg["']/,
  );
  assert.doesNotMatch(html, /Signed build coming/);
  assert.doesNotMatch(html, /href=["'](?:file:|\/Users\/|\/tmp\/)/i);
});

test("publishes the required brand and social assets", async () => {
  await Promise.all([
    access(new URL("../public/spick-mark.png", import.meta.url)),
    access(new URL("../public/og.png", import.meta.url)),
  ]);

  const response = await render();
  const html = await response.text();
  assert.match(html, /property=["']og:image["']/i);
  assert.match(html, /content=["']http:\/\/localhost(?::3000)?\/og\.png["']/i);
  assert.match(html, /rel=["']icon["'][^>]+spick-mark\.png/i);
  assert.doesNotMatch(html, /_vinext\/image\?url=/i);
});
