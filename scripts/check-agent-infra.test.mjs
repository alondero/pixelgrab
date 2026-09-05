import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { checkAlias, checkDocuments } from "./check-agent-infra.mjs";

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "pixelgrab-agent-infra-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return {
    root,
    write(path, content) {
      const target = join(root, path);
      mkdirSync(dirname(target), { recursive: true });
      writeFileSync(target, content);
    },
  };
}

test("resolves nested local links and ignores external links and anchors", (t) => {
  const f = fixture(t);
  f.write("CLAUDE.md", "# Guide");
  f.write("docs/guide.md", "[root](../CLAUDE.md#guide) [site](https://example.com) [here](#x)");
  assert.deepEqual(checkDocuments(f.root, ["docs/guide.md"]), []);
});

test("rejects missing documents, broken links, and repository escapes", (t) => {
  const f = fixture(t);
  f.write("CLAUDE.md", "[missing](lost.md) [escape](../outside.md)");
  const errors = checkDocuments(f.root, ["CLAUDE.md", "docs/missing.md"]);
  assert.equal(errors.length, 3);
  assert.ok(errors.some((error) => error.startsWith("Broken link:")));
  assert.ok(errors.some((error) => error.startsWith("Link escapes repository:")));
  assert.ok(errors.some((error) => error.startsWith("Missing document:")));
});

test("requires discoverable skill metadata", (t) => {
  const f = fixture(t);
  f.write("SKILL.md", "# No metadata");
  assert.equal(checkDocuments(f.root, ["SKILL.md"]).length, 1);
  f.write("SKILL.md", "---\nname: example\ndescription: Test skill.\n---\n");
  assert.deepEqual(checkDocuments(f.root, ["SKILL.md"]), []);
});

test("accepts Windows Git placeholders for Claude-canonical aliases", (t) => {
  const f = fixture(t);
  f.write("CLAUDE.md", "# Guide");
  f.write(
    ".claude/skills/pixelgrab-change/SKILL.md",
    "---\nname: example\ndescription: Test skill.\n---\n",
  );
  assert.equal(checkAlias(f.root).errors.length, 2);
  f.write("AGENTS.md", "CLAUDE.md");
  f.write(".agents/skills", "../.claude/skills");
  assert.deepEqual(checkAlias(f.root).errors, []);
  assert.equal(checkAlias(f.root).warnings.length, 2);
  f.write("AGENTS.md", "# Duplicated guide");
  assert.equal(checkAlias(f.root).errors.length, 1);
});
