// Guard: a locale tag is data, never a literal in UI or server code.
//
// The campaign is multi-locale; `locales.json` is the registry and every reader
// derives its tags from it. A `"es"` written into a page, a runtime module or a
// Rust match arm is the failure that makes adding the next language a code
// change instead of a catalog drop, and it is invisible until that day arrives.
//
// Two literals are allowed, each by name and exactly once: the fallback tag
// (`FALLBACK_LOCALE`, the one constant every module imports) and the generated
// pseudo-locale (`PSEUDO_LOCALE`, an overflow-check tool that is deliberately
// never in `locales.json`). A third allowance covers a real `es` that is not a
// locale: the ISO 3166 country code for Spain in the LTS country list. It is a
// count, so a second `es` in that file is a finding.
//
// The catalog/locales pairing ("every catalog is listed, every listed locale has
// a catalog") is `build-catalogs.mjs`'s job and is tested in
// `build-catalogs.test.mjs`; this file does not re-implement it.

import assert from "node:assert/strict";
import {existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {dirname, join, relative, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";
import {scanLiterals} from "../../i18n/candidates.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const JS_BUILD_DIR = resolve(HERE, "../..");
const REAL_ROOT = resolve(JS_BUILD_DIR, "..");

const FALLBACK_NAME = "FALLBACK_LOCALE";
const PSEUDO_NAME = "PSEUDO_LOCALE";

// A literal that is a real word in one place and a locale tag in another. Keyed
// by file and literal, valued by how many occurrences are that other thing.
const NOT_A_LOCALE = new Map([
    [
        "js_build/src/lts_trial.js#es",
        {count: 1, reason: "'es' is the ISO 3166 country code for Spain in the LTS country list"},
    ],
]);

const roots = [];

test.after(() => {
    for (const root of roots) rmSync(root, {recursive: true, force: true});
});

function walk(directory, suffix) {
    const files = [];
    for (const entry of readdirSync(directory, {withFileTypes: true})) {
        const path = resolve(directory, entry.name);
        if (entry.isDirectory()) files.push(...walk(path, suffix));
        else if (path.endsWith(suffix)) files.push(path);
    }
    return files;
}

function lineIndex(text) {
    const offsets = [0];
    for (let index = 0; index < text.length; index += 1) {
        if (text[index] === "\n") offsets.push(index + 1);
    }
    return (index) => {
        let line = 0;
        while (line + 1 < offsets.length && offsets[line + 1] <= index) line += 1;
        return line + 1;
    };
}

/** Skips a Rust comment, string, char literal or raw string at `index`. */
function skipRustToken(source, index) {
    const character = source[index];
    if (character === "/" && source[index + 1] === "/") {
        const end = source.indexOf("\n", index);
        return end === -1 ? source.length : end;
    }
    if (character === "/" && source[index + 1] === "*") {
        let depth = 0;
        let cursor = index;
        while (cursor < source.length) {
            if (source.startsWith("/*", cursor)) {
                depth += 1;
                cursor += 2;
                continue;
            }
            if (source.startsWith("*/", cursor)) {
                depth -= 1;
                cursor += 2;
                if (depth === 0) return cursor;
                continue;
            }
            cursor += 1;
        }
        return source.length;
    }
    const raw = /^(?:b?r)(#*)"/.exec(source.slice(index, index + 40));
    if (raw && (index === 0 || !/[\w$]/.test(source[index - 1]))) {
        const terminator = `"${raw[1]}`;
        const start = index + raw[0].length;
        const end = source.indexOf(terminator, start);
        return end === -1 ? source.length : end + terminator.length;
    }
    if (character === '"' || (character === "b" && source[index + 1] === '"')) {
        let cursor = character === "b" ? index + 2 : index + 1;
        while (cursor < source.length) {
            if (source[cursor] === "\\") {
                cursor += 2;
                continue;
            }
            if (source[cursor] === '"') return cursor + 1;
            cursor += 1;
        }
        return source.length;
    }
    if (character === "'") {
        // A char literal closes on the same line; a lifetime (`'a`) does not.
        if (source[index + 1] === "\\") return source.indexOf("'", index + 2) + 1 || source.length;
        if (source[index + 2] === "'") return index + 3;
        return index + 1;
    }
    return index;
}

/** The source with every `#[cfg(test)]` item blanked, newlines preserved. */
function maskTestItems(source) {
    const characters = [...source];
    for (const match of source.matchAll(/#\[cfg\(test\)\]/g)) {
        let open = match.index + match[0].length;
        while (open < source.length && source[open] !== "{") open = skipRustToken(source, open) + 1;
        if (open >= source.length) continue;
        let depth = 0;
        let close = open;
        while (close < source.length) {
            close = skipRustToken(source, close);
            if (source[close] === "{") depth += 1;
            else if (source[close] === "}") {
                depth -= 1;
                if (depth === 0) break;
            }
            close += 1;
        }
        for (let index = match.index; index < Math.min(close + 1, source.length); index += 1) {
            if (characters[index] !== "\n") characters[index] = " ";
        }
    }
    return characters.join("");
}

/** Every string literal in Rust source, with its line. */
function rustLiterals(source) {
    const lineOf = lineIndex(source);
    const literals = [];
    let index = 0;
    while (index < source.length) {
        const character = source[index];
        const end = skipRustToken(source, index);
        if (end !== index) {
            const isString =
                character === '"' ||
                (character === "b" && source[index + 1] === '"') ||
                /^(?:b?r)#*"/.test(source.slice(index, index + 40));
            if (isString) {
                const value = source
                    .slice(index, end)
                    .replace(/^b?r?#*"/, "")
                    .replace(/"#*$/, "");
                literals.push({value, line: lineOf(index)});
            }
            index = end;
            continue;
        }
        index += 1;
    }
    return literals;
}

/** Locale tags and their regional forms (`es`, `es-419`, `es_MX`). */
function tagMatcher(tags) {
    return (literal) => {
        const value = literal.trim().toLowerCase();
        return tags.some((tag) => value === tag || value.startsWith(`${tag}-`) || value.startsWith(`${tag}_`));
    };
}

/**
 * Scans one tree, reporting locale literals that the allowances do not cover and
 * the single-definition hits the allowances require.
 */
function scanTree(root) {
    const catalogDir = resolve(root, "static2/i18n");
    const locales = JSON.parse(readFileSync(resolve(catalogDir, "locales.json"), "utf8"));
    const isTag = tagMatcher(locales.map((locale) => locale.tag));

    const runtimeDir = resolve(root, "js_build/src/i18n");
    const entries = readFileSync(resolve(root, "js_build/entrypoints.txt"), "utf8")
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean)
        .map((name) => resolve(root, "js_build/src", name));
    const files = [
        ...readdirSync(runtimeDir)
            .filter((name) => /\.(?:js|mjs)$/.test(name) && !/\.test\./.test(name))
            .map((name) => resolve(runtimeDir, name)),
        ...entries,
        ...readdirSync(resolve(root, "js_build/i18n"))
            .filter((name) => name.endsWith(".mjs"))
            .map((name) => resolve(root, "js_build/i18n", name)),
    ];

    const findings = [];
    const hits = {[FALLBACK_NAME]: 0, [PSEUDO_NAME]: 0};
    const allowances = new Map([...NOT_A_LOCALE].map(([key, value]) => [key, {...value}]));

    const report = (file, line, literal) =>
        findings.push(
            `${relative(root, file)}:${line}: ${JSON.stringify(literal)} is a locale tag; ` +
                "read it from locales.json or from the shared constant",
        );

    for (const file of files) {
        const source = readFileSync(file, "utf8");
        const lines = source.split("\n");
        for (const literal of scanLiterals(source).literals) {
            if (!isTag(literal.value)) continue;
            const declared = lines[literal.line - 1] ?? "";
            const relativePath = relative(root, file);
            if (relativePath === "js_build/src/i18n/locale.mjs" && declared.includes(`const ${FALLBACK_NAME} =`)) {
                hits[FALLBACK_NAME] += 1;
                continue;
            }
            if (relativePath === "js_build/i18n/build-catalogs.mjs" && declared.includes(`const ${PSEUDO_NAME} =`)) {
                hits[PSEUDO_NAME] += 1;
                continue;
            }
            const key = `${relativePath}#${literal.value}`;
            const allowance = allowances.get(key);
            if (allowance && allowance.count > 0) {
                allowance.count -= 1;
                continue;
            }
            report(file, literal.line, literal.value);
        }
    }

    // The tag literals this guard depends on must be found: a rename would
    // otherwise turn the allowance into a silent hole.
    for (const [name, declared] of [
        [FALLBACK_NAME, "js_build/src/i18n/locale.mjs"],
        [PSEUDO_NAME, "js_build/i18n/build-catalogs.mjs"],
    ]) {
        if (hits[name] !== 1) {
            findings.push(`${declared}: const ${name} must be declared exactly once with a locale tag (found ${hits[name]})`);
        }
    }
    for (const [key, allowance] of allowances) {
        // Only the tree that holds the file can judge the count: a fixture that
        // does not ship `lts_trial.js` cannot say its allowance is stale.
        if (allowance.count === 0) continue;
        if (!existsSync(resolve(root, key.slice(0, key.indexOf("#"))))) continue;
        findings.push(`${key}: ${allowance.reason}, but ${allowance.count} occurrence(s) went unmatched; update the allowance`);
    }

    const rustFindings = scanRust(root, isTag);
    findings.push(...rustFindings);

    return findings;
}

/** Rust: no tag literal outside `locale.rs`'s default constant, test code ignored. */
function scanRust(root, isTag) {
    const findings = [];
    let defaults = 0;
    for (const file of walk(root, ".rs")) {
        const source = maskTestItems(readFileSync(file, "utf8"));
        const lines = source.split("\n");
        for (const literal of rustLiterals(source)) {
            if (!isTag(literal.value)) continue;
            const declared = lines[literal.line - 1] ?? "";
            if (relative(root, file) === "locale.rs" && declared.includes("const DEFAULT_UI_LOCALE")) {
                defaults += 1;
                continue;
            }
            findings.push(
                `${relative(root, file)}:${literal.line}: ${JSON.stringify(literal.value)} is a locale tag; ` +
                    "resolve it through locale.rs instead",
            );
        }
    }
    if (defaults !== 1) {
        findings.push(`locale.rs: const DEFAULT_UI_LOCALE must be declared exactly once with a locale tag (found ${defaults})`);
    }
    return findings;
}

/** A node_manager-shaped tree with one page entrypoint. */
function fixture({
    entrypoint = 'export const label = "Tree";\n',
    entries = ["page.js"],
    files = {},
    locales = [{tag: "en", name: "English", dir: "ltr"}, {tag: "es", name: "Español", dir: "ltr"}],
} = {}) {
    const root = mkdtempSync(join(tmpdir(), "lqos-locale-guard-"));
    roots.push(root);
    mkdirSync(resolve(root, "static2/i18n"), {recursive: true});
    mkdirSync(resolve(root, "js_build/i18n"), {recursive: true});
    mkdirSync(resolve(root, "js_build/src/i18n"), {recursive: true});
    writeFileSync(resolve(root, "static2/i18n/locales.json"), JSON.stringify(locales, null, 2));
    writeFileSync(resolve(root, "js_build/entrypoints.txt"), `${entries.join("\n")}\n`);
    writeFileSync(resolve(root, "js_build/src/page.js"), entrypoint);
    for (const [name, source] of Object.entries(files)) writeFileSync(resolve(root, "js_build/src", name), source);
    writeFileSync(
        resolve(root, "js_build/src/i18n/locale.mjs"),
        `export const FALLBACK_LOCALE = "${locales[0].tag}";\nexport function locale() { return FALLBACK_LOCALE; }\n`,
    );
    writeFileSync(
        resolve(root, "js_build/i18n/build-catalogs.mjs"),
        `const PSEUDO_LOCALE = "${locales[0].tag}-XA";\nexport default PSEUDO_LOCALE;\n`,
    );
    writeFileSync(
        resolve(root, "locale.rs"),
        `pub(crate) const DEFAULT_UI_LOCALE: &str = "${locales[0].tag}";\n` +
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn spanish() {\n        assert_eq!(ui_locale(), \"es\");\n    }\n}\n",
    );
    return root;
}

test("the shipped tree spells no locale tag outside the single fallback constant", () => {
    const findings = scanTree(REAL_ROOT);
    assert.deepEqual(findings, [], findings.join("\n"));
});

test("a locale literal in a page entrypoint is reported with its file and literal", () => {
    const root = fixture({entrypoint: 'export const label = "Tree";\nconst language = "es";\n'});
    const findings = scanTree(root);
    assert.equal(findings.length, 1, findings.join("\n"));
    assert.match(findings[0], /js_build\/src\/page\.js:2: "es" is a locale tag/);
});

test("a locale literal in a runtime module is reported too", () => {
    const root = fixture();
    writeFileSync(resolve(root, "js_build/src/i18n/switcher.mjs"), 'export const DEFAULT = "es-MX";\n');
    const findings = scanTree(root);
    assert.equal(findings.length, 1, findings.join("\n"));
    assert.match(findings[0], /js_build\/src\/i18n\/switcher\.mjs:1: "es-MX" is a locale tag/);
});

test("a locale literal in Rust production code is reported; test code is ignored", () => {
    const root = fixture();
    writeFileSync(
        resolve(root, "template.rs"),
        'fn language() -> &\'static str {\n    "es"\n}\n' +
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn english() {\n        assert_eq!(language(), \"en\");\n    }\n}\n",
    );
    const findings = scanTree(root);
    assert.equal(findings.length, 1, findings.join("\n"));
    assert.match(findings[0], /template\.rs:2: "es" is a locale tag/);
});

test("moving the fallback constant out of its declaration is reported", () => {
    const root = fixture();
    writeFileSync(
        resolve(root, "js_build/src/i18n/locale.mjs"),
        'export const FALLBACK_LOCALE = "en";\nexport const ENGLISH = "en";\n',
    );
    const findings = scanTree(root);
    assert.equal(findings.length, 1, findings.join("\n"));
    assert.match(findings[0], /locale\.mjs:2: "en" is a locale tag/);
});

test("a country code that looks like a locale tag is allowed once, and a second one is not", () => {
    // `js_build/src/lts_trial.js` carries ISO 3166 codes; `es` is Spain. The
    // allowance is per file, per literal and counted.
    const one = fixture({
        entries: ["lts_trial.js"],
        files: {"lts_trial.js": 'export const COUNTRIES = [{code: "es", name: "Spain"}];\n'},
    });
    assert.deepEqual(scanTree(one), []);

    const two = fixture({
        entries: ["lts_trial.js"],
        files: {
            "lts_trial.js": 'export const COUNTRIES = [{code: "es", name: "Spain"}, {code: "es", name: "España"}];\n',
        },
    });
    const findings = scanTree(two);
    assert.equal(findings.length, 1, findings.join("\n"));
    assert.match(findings[0], /js_build\/src\/lts_trial\.js:1: "es" is a locale tag/);
});

test("the same literal outside the allowed file is reported", () => {
    const root = fixture({entrypoint: 'export const COUNTRIES = [{code: "es", name: "Spain"}];\n'});
    const findings = scanTree(root);
    assert.equal(findings.length, 1, findings.join("\n"));
    assert.match(findings[0], /js_build\/src\/page\.js:1: "es" is a locale tag/);
});
