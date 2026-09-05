import { existsSync, lstatSync, readFileSync, readlinkSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const documents = [
  "CLAUDE.md",
  ".claude/skills/pixelgrab-change/SKILL.md",
  "docs/agents/verification.md",
  "docs/agents/infrastructure.md",
  "docs/agents/2026-09-05-audit.md",
  "docs/agents/implementation-reference.md",
];

export const aliases = [
  { alias: "AGENTS.md", target: "CLAUDE.md" },
  {
    alias: ".agents/skills/pixelgrab-change/SKILL.md",
    target: ".claude/skills/pixelgrab-change/SKILL.md",
  },
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

/** Accept Git symlinks and the target-text files used by Windows checkouts. */
export function checkAliases(root, expectedAliases = aliases) {
  const errors = [];
  const warnings = [];
  for (const { alias, target } of expectedAliases) {
    const aliasPath = resolve(root, alias);
    const targetPath = resolve(root, target);
    const relativeTarget = relative(dirname(aliasPath), targetPath).replaceAll("\\", "/");
    try {
      const stat = lstatSync(aliasPath);
      if (stat.isSymbolicLink()) {
        const actualTarget = resolve(dirname(aliasPath), readlinkSync(aliasPath));
        if (actualTarget !== targetPath) {
          errors.push(`Alias points to the wrong target: ${alias} -> ${readlinkSync(aliasPath)}`);
        }
      } else if (
        readFileSync(aliasPath, "utf8").replaceAll("\r\n", "\n").trim() !== relativeTarget
      ) {
        errors.push(`Alias must be a symlink to ${target}: ${alias}`);
      } else {
        warnings.push(
          `${alias} is a symlink placeholder; enable symlink support for a native link.`,
        );
      }
      if (!existsSync(targetPath)) errors.push(`Alias target is missing: ${alias} -> ${target}`);
    } catch {
      errors.push(`Missing or unreadable alias: ${alias}`);
    }
  }
  return { errors, warnings };
}

export function checkAlias(root) {
  return checkAliases(root);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const aliasCheck = checkAliases(repositoryRoot);
  const errors = [...checkDocuments(repositoryRoot, documents), ...aliasCheck.errors];
  for (const warning of aliasCheck.warnings) console.warn(warning);
  for (const error of errors) console.error(error);
  if (errors.length > 0) process.exitCode = 1;
  else console.info("Agent infrastructure check passed.");
}
