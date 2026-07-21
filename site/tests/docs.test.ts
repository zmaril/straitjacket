import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";

const DOCS_DIR = join(import.meta.dir, "..", "content", "docs");

/** Recursively collect every `meta.json` under the docs tree. */
function metaFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...metaFiles(full));
    else if (entry.name === "meta.json") out.push(full);
  }
  return out;
}

const metas = metaFiles(DOCS_DIR);

describe("docs meta.json", () => {
  test("there is at least one meta.json", () => {
    expect(metas.length).toBeGreaterThan(0);
  });

  for (const meta of metas) {
    const rel = meta.slice(DOCS_DIR.length + 1);

    test(`${rel} parses and every plain page entry resolves`, () => {
      const parsed = JSON.parse(readFileSync(meta, "utf8"));
      const pages: unknown = parsed.pages ?? [];
      expect(Array.isArray(pages)).toBe(true);

      const here = dirname(meta);
      for (const page of pages as unknown[]) {
        // Only validate plain slugs; skip Fumadocs' special tokens
        // (separators "---x---", links "[a](b)", rest "...", extract "!x", "z...").
        if (typeof page !== "string" || !/^[a-z0-9-]+$/i.test(page)) continue;
        const asFile = join(here, `${page}.mdx`);
        const asDir = join(here, page);
        expect(existsSync(asFile) || existsSync(asDir)).toBe(true);
      }
    });
  }
});
