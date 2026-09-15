/// <reference types="node" />
import { describe, it, expect } from 'vitest';
import { readFileSync, readdirSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';
import en from './en.json';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const srcDir = join(__dirname, '../../..');

type MessageKey = keyof typeof en;

/**
 * Recursively find all .svelte and .ts/.tsx files
 */
function findSourceFiles(dir: string, exclude: Set<string> = new Set()): string[] {
  const files: string[] = [];
  const entries = readdirSync(dir, { withFileTypes: true });

  for (const entry of entries) {
    const fullPath = join(dir, entry.name);

    // Skip node_modules, dist, and other build artifacts
    if (
      entry.name === 'node_modules' ||
      entry.name === 'dist' ||
      entry.name === '.svelte-kit' ||
      exclude.has(fullPath)
    ) {
      continue;
    }

    if (entry.isDirectory()) {
      files.push(...findSourceFiles(fullPath, exclude));
    } else if (entry.isFile() && /\.(svelte|ts|tsx)$/.test(entry.name)) {
      files.push(fullPath);
    }
  }

  return files;
}

/**
 * Extract all `t("...")` calls from a file
 */
function extractTCalls(content: string): string[] {
  // Match t("key") or t('key'), but not things like getAttribute("data-testid")
  // by requiring a word character before the t (or start of line/space) and not having HTML context
  const matches = content.match(/(?<![a-zA-Z0-9_])t\(\s*["']([^"']+)["']\s*(?:,\s*[^)]*?)?\)/g);
  if (!matches) return [];
  const result: string[] = [];
  for (const m of matches) {
    const match = m.match(/t\(\s*["']([^"']+)["']/);
    if (match && match[1]) {
      result.push(match[1]);
    }
  }
  return result;
}

/**
 * Extract all `tDynamic("prefix...")` calls to check prefixes
 */
function extractTDynamicPrefixes(content: string): string[] {
  // Look for tDynamic calls where the key might be dynamic
  // e.g., tDynamic(`error.${code}`)
  // We extract the prefix: "error."
  const matches = content.match(/tDynamic\(\s*[`"].*?[`"]\s*[,\)]/g);
  if (!matches) return [];

  const prefixes: string[] = [];
  for (const match of matches) {
    // Handle template literals: tDynamic(`error.${...}`)
    const templateMatch = match.match(/tDynamic\(\s*`([^$]*)\$/);
    if (templateMatch && templateMatch[1]) {
      prefixes.push(templateMatch[1]);
    }
    // Handle string concatenation: tDynamic("error." + code)
    const concatMatch = match.match(/tDynamic\(\s*["']([^"']*)['"]\s*\+/);
    if (concatMatch && concatMatch[1]) {
      prefixes.push(concatMatch[1]);
    }
  }
  return prefixes;
}

/**
 * Extract placeholder names from template strings in a file
 * e.g., t("key.with.{placeholder}")
 */
function extractPlaceholders(content: string): Map<string, Set<string>> {
  const placeholders = new Map<string, Set<string>>();
  const tCalls = content.match(/t\(\s*["']([^"']+)["']\s*,\s*({[^}]*})/g);
  if (!tCalls) return placeholders;

  for (const call of tCalls) {
    const match = call.match(/t\(\s*["']([^"']+)["']\s*,\s*({.*?})\s*\)/);
    if (!match || !match[1] || !match[2]) continue;

    const key = match[1];
    const paramsStr = match[2];
    const paramMatches = paramsStr.match(/(\w+):/g);
    const paramNames = (paramMatches || []).map((p) => p.slice(0, -1));
    placeholders.set(key, new Set(paramNames));
  }

  return placeholders;
}

describe('i18n lint', () => {
  it('every t("...") key exists in en.json', () => {
    const sourceFiles = findSourceFiles(srcDir).filter(
      (f) => !f.includes('node_modules') && !f.includes('i18n.test.ts') && !f.includes('.test.ts')
    );

    const missingKeys: Array<{ key: string; file: string }> = [];
    const usedKeys = new Set<string>();

    for (const file of sourceFiles) {
      const content = readFileSync(file, 'utf-8');
      const keys = extractTCalls(content);

      for (const key of keys) {
        usedKeys.add(key);
        if (!(key in en)) {
          missingKeys.push({ key, file });
        }
      }
    }

    if (missingKeys.length > 0) {
      const grouped = missingKeys.reduce(
        (acc, { key, file }) => {
          if (!acc[key]) acc[key] = [];
          acc[key].push(file);
          return acc;
        },
        {} as Record<string, string[]>
      );

      const msg = Object.entries(grouped)
        .map(([key, files]) => `  "${key}": ${files.length} files\n    ${files.join('\n    ')}`)
        .join('\n');

      throw new Error(`Missing keys in en.json:\n${msg}`);
    }
  });

  it('every tDynamic() prefix has matching keys in en.json', () => {
    const sourceFiles = findSourceFiles(srcDir).filter(
      (f) => !f.includes('node_modules') && !f.includes('i18n.test.ts') && !f.includes('.test.ts')
    );

    const missingPrefixes: Array<{ prefix: string; file: string }> = [];

    for (const file of sourceFiles) {
      const content = readFileSync(file, 'utf-8');
      const prefixes = extractTDynamicPrefixes(content);

      for (const prefix of prefixes) {
        // Check if at least one key with this prefix exists
        const hasPrefix = Object.keys(en).some((k) => k.startsWith(prefix));
        if (!hasPrefix) {
          missingPrefixes.push({ prefix, file });
        }
      }
    }

    if (missingPrefixes.length > 0) {
      const grouped = missingPrefixes.reduce(
        (acc, { prefix, file }) => {
          if (!acc[prefix]) acc[prefix] = [];
          acc[prefix].push(file);
          return acc;
        },
        {} as Record<string, string[]>
      );

      const msg = Object.entries(grouped)
        .map(
          ([prefix, files]) => `  "${prefix}*": ${files.length} files\n    ${files.join('\n    ')}`
        )
        .join('\n');

      throw new Error(`tDynamic() prefixes with no matching keys in en.json:\n${msg}`);
    }
  });

  it('placeholder params match template variables in keys', () => {
    const sourceFiles = findSourceFiles(srcDir).filter(
      (f) => !f.includes('node_modules') && !f.includes('i18n.test.ts') && !f.includes('.test.ts')
    );

    const mismatches: Array<{
      key: string;
      templateVars: string[];
      providedParams: string[];
      file: string;
    }> = [];

    for (const file of sourceFiles) {
      const content = readFileSync(file, 'utf-8');
      const placeholders = extractPlaceholders(content);

      for (const [key, providedParams] of placeholders.entries()) {
        const templateText = en[key as MessageKey];
        if (!templateText) continue;

        const templateVars = (templateText.match(/{(\w+)}/g) || []).map((m) =>
          m.slice(1, -1)
        );

        const provided = Array.from(providedParams);
        const missing = templateVars.filter((v) => !providedParams.has(v));
        const extra = provided.filter((p) => !templateVars.includes(p));

        if (missing.length > 0 || extra.length > 0) {
          mismatches.push({
            key,
            templateVars,
            providedParams: provided,
            file,
          });
        }
      }
    }

    if (mismatches.length > 0) {
      const msg = mismatches
        .map(
          ({ key, templateVars, providedParams, file }) =>
            `  "${key}": expected {${templateVars.join(', ')}}, got {${providedParams.join(', ')}}\n    ${file}`
        )
        .join('\n');

      throw new Error(`Placeholder mismatches:\n${msg}`);
    }
  });

  it('report unused keys in en.json (informational only)', () => {
    const sourceFiles = findSourceFiles(srcDir).filter(
      (f) => !f.includes('node_modules') && !f.includes('i18n.test.ts') && !f.includes('.test.ts')
    );

    const usedKeys = new Set<string>();
    const dynamicPrefixes = new Set<string>();

    for (const file of sourceFiles) {
      const content = readFileSync(file, 'utf-8');

      // Collect all t() keys
      const tKeys = extractTCalls(content);
      tKeys.forEach((k) => usedKeys.add(k));

      // Collect all tDynamic() prefixes
      const tDynamicPrefixes = extractTDynamicPrefixes(content);
      tDynamicPrefixes.forEach((p) => dynamicPrefixes.add(p));
    }

    // Also check for any keys that are likely to be dynamically accessed
    // by looking for patterns like error.*, notice.*
    const likelyDynamic = Object.keys(en).filter((k) => {
      for (const prefix of dynamicPrefixes) {
        if (k.startsWith(prefix)) return true;
      }
      return false;
    });

    likelyDynamic.forEach((k) => usedKeys.add(k));

    // Find unused keys
    const unusedKeys = Object.keys(en).filter((k) => !usedKeys.has(k));

    if (unusedKeys.length > 0) {
      console.log(`\n⚠️  Unused keys in en.json (${unusedKeys.length}):`);
      unusedKeys.forEach((k) => console.log(`   ${k}`));
    }

    // This test always passes; it's informational
    expect(true).toBe(true);
  });
});
