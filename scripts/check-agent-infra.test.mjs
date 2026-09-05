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
  f.write("AGENTS.md", "# Guide");
  f.write("docs/guide.md", "[root](../AGENTS.md#guide) [site](https://example.com) [here](#x)");
  assert.deepEqual(checkDocuments(f.root, ["docs/guide.md"]), []);
});

test("rejects missing documents, broken links, and repository escapes", (t) => {
  const f = fixture(t);
  f.write("AGENTS.md", "[missing](lost.md) [escape](../outside.md)");
  const errors = checkDocuments(f.root, ["AGENTS.md", "docs/missing.md"]);
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

test("accepts a Windows Git placeholder with a warning but rejects a second guide", (t) => {
  const f = fixture(t);
  f.write("AGENTS.md", "# Guide");
  assert.equal(checkAlias(f.root).errors.length, 1);
  f.write("CLAUDE.md", "AGENTS.md");
  assert.deepEqual(checkAlias(f.root).errors, []);
  assert.equal(checkAlias(f.root).warnings.length, 1);
  f.write(
    "CLAUDE.md",
    "Read and follow [AGENTS.md](AGENTS.md) before working in this repository.\nIt is the canonical project instruction file; edit it instead of this forwarding file.\n",
  );
  assert.deepEqual(checkAlias(f.root), { errors: [], warnings: [] });
  f.write("CLAUDE.md", "# Duplicated guide");
  assert.equal(checkAlias(f.root).errors.length, 1);
});
