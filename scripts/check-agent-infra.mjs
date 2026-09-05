import { existsSync, lstatSync, readFileSync, readlinkSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const forwardingText =
  "Read and follow [AGENTS.md](AGENTS.md) before working in this repository.\nIt is the canonical project instruction file; edit it instead of this forwarding file.";

export const documents = [
  "AGENTS.md",
  ".agents/skills/pixelgrab-change/SKILL.md",
  ".claude/skills/pixelgrab-change/SKILL.md",
  "docs/agents/verification.md",
  "docs/agents/infrastructure.md",
  "docs/agents/2026-09-05-audit.md",
  "docs/agents/implementation-reference.md",
];

/** Check the maintained Markdown subset: inline local file links, not anchors. */
export function checkDocuments(root, paths) {
  const errors = [];
  for (const path of paths) {
    const absolute = resolve(root, path);
    if (!existsSync(absolute)) {
      errors.push(`Missing document: ${path}`);
      continue;
    }
    const content = readFileSync(absolute, "utf8");
    if (
      path.endsWith("SKILL.md") &&
      !/^---\r?\nname: .+\r?\ndescription: .+\r?\n---/u.test(content)
    ) {
      errors.push(`Missing skill name/description frontmatter: ${path}`);
    }
    for (const [, link] of content.matchAll(/\[[^\]\n]*\]\(([^)\n]+)\)/gu)) {
      if (/^(?:https?:|mailto:|#)/u.test(link)) continue;
      const target = resolve(dirname(absolute), link.split("#")[0]);
      const local = relative(root, target);
      if (isAbsolute(local) || local === ".." || local.startsWith(`..${sep}`)) {
        errors.push(`Link escapes repository: ${path} -> ${link}`);
      } else if (!existsSync(target)) {
        errors.push(`Broken link: ${path} -> ${link}`);
      }
    }
  }
  return errors;
}

/** Accept a symlink or the portable forwarding file without duplicate rules. */
export function checkAlias(root) {
  const alias = resolve(root, "CLAUDE.md");
  try {
    if (lstatSync(alias).isSymbolicLink()) {
      return readlinkSync(alias) === "AGENTS.md"
        ? { errors: [], warnings: [] }
        : { errors: ["CLAUDE.md must link to relative AGENTS.md"], warnings: [] };
    }
    const content = readFileSync(alias, "utf8").replaceAll("\r\n", "\n").trim();
    if (content === forwardingText) return { errors: [], warnings: [] };
    if (content === "AGENTS.md") {
      return {
        errors: [],
        warnings: [
          "CLAUDE.md is a Git symlink placeholder; read AGENTS.md directly or restore symlink support.",
        ],
      };
    }
    return {
      errors: [
        "CLAUDE.md duplicates or diverges from AGENTS.md; restore the forwarding file or symlink",
      ],
      warnings: [],
    };
  } catch {
    return { errors: ["Missing or unreadable CLAUDE.md alias"], warnings: [] };
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const alias = checkAlias(repositoryRoot);
  const errors = [...checkDocuments(repositoryRoot, documents), ...alias.errors];
  for (const warning of alias.warnings) console.warn(warning);
  for (const error of errors) console.error(error);
  if (errors.length > 0) process.exitCode = 1;
  else console.info("Agent infrastructure check passed.");
}
