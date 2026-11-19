// Candidate scanner behind `extract.mjs --inventory`.
//
// This is the heuristic from `I8N.md` §1.3, in code: HTML text nodes and the
// title/aria-label/placeholder/alt values, plus JS string and template literals
// with `${…}` and markup removed. What it finds is a *candidate*, not a fact:
// roughly a fifth of the JS hits are identifiers, units or paths. The point is a
// denominator a unit can prove movement against, so the filter errs towards
// keeping a doubtful literal (a visible false positive costs one review line; a
// dropped real string makes the target lie).
//
// A literal the team has judged not to be text is argued out of the count with a
// `translatable: false` marker that also gives the reason, in the comment next to
// it:
//
//     // translatable: false — "Mbps" is a unit, not a label
//
// The marker names the literal in quotes, so it works in HTML comments too, and
// a marker naming a string the file no longer contains is reported as stale.

const MARKER = /translatable\s*:\s*false\b/i;
const MARKER_LITERAL = /"([^"]+)"|'([^']+)'/g;
const LETTERS = /\p{L}/gu;
const TAG = /<[^>]*>/g;
const PLACEHOLDER = /\$\{[^}]*\}/g;
// A start or end tag, with the attribute text in group 3. A quoted `>` belongs
// to the tag, so `title="a > b"` does not split it.
const START_OR_END_TAG = /<(\/?)([a-zA-Z][\w:-]*)((?:"[^"]*"|'[^']*'|[^>"'])*)>/g;
const TRANSLATABLE_ATTRIBUTES = ["title", "aria-label", "placeholder", "alt"];
// Elements that never hold a text node, so an open tag does not open a scope.
const VOID_ELEMENTS = new Set([
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta",
    "param", "source", "track", "wbr",
]);
const ENTITIES = new Map([
    ["&amp;", "&"],
    ["&lt;", "<"],
    ["&gt;", ">"],
    ["&quot;", '"'],
    ["&#39;", "'"],
    ["&apos;", "'"],
    ["&nbsp;", " "],
]);

// `I8N.md` §1.2: units, protocol tokens and product names are not catalog text.
// Only the capitalised or mixed-case ones need naming; a lowercase single token
// is already filtered as an identifier, and anything shorter than three
// characters never reaches this test. Entries are matched lower-cased.
const NOT_TEXT = new Set([
    "api", "apis", "arp", "bgp", "bps", "cake", "cpu", "cpus",
    "csv", "css", "ddos", "dhcp", "dns", "dpdk", "fq_codel", "ftp",
    "gib", "gbps", "ghz", "gpu", "gpus", "gtm", "html", "htb", "http",
    "https", "icmp", "ids", "ips", "ipv4", "ipv6", "json",
    "kbps", "kib", "khz", "lqosd", "lqusers", "lts", "mac", "mbps",
    "mib", "mhz", "mtu", "nic", "nics", "ntp", "p50", "p95", "p99",
    "pps", "ram", "rtt", "smtp", "snmp", "ssh", "ssl", "tcp", "tls", "udp",
    "url", "urls", "uuid", "vlan", "vxlan", "wasm", "xdp", "xml", "yaml",
    "get", "post", "put", "patch", "delete", "head", "options",
    "libreqos", "insight", "ask libby", "stormguard", "splynx", "uisp", "visp",
    "netzur", "wispgate", "powercode", "sonar", "radius", "uplot",
]);

const URL_OR_PATH = [
    /^[a-z][a-z0-9+.-]*:\/\//i,
    /^www\./i,
    /^\/\//,
    /^\//,
    /^\.{1,2}\//,
    /^[\w.@-]+\.(?:js|mjs|json|css|html?|svg|png|jpe?g|gif|webp|ico|woff2?|ttf|csv|txt|map|wasm)$/i,
    /^[\w.-]+\/[\w./-]+$/,
    /\.(?:html?|js|mjs|json|css|svg|png|gif)(?:[?#]|$)/,
];

/** A normalized candidate line, or `null` when the literal is not text-like. */
function classifyText(text) {
    if (text.length < 3) return null;
    if (countLetters(text) < 2) return null;
    const lower = text.toLowerCase();
    if (NOT_TEXT.has(lower)) return null;
    // A template token (`%%TITLE%%`) or a fragment of markup split by an
    // interpolation: neither is text on screen.
    if (/^%%.*%%$/.test(text)) return null;
    if (/<[A-Za-z/!]/.test(text)) return null;
    if (URL_OR_PATH.some((pattern) => pattern.test(text))) return null;
    // An element id, an anchor or a colour literal.
    if (/^#[A-Za-z0-9_-]+$/.test(text)) return null;
    // A catalog key or a `data-*` name: dotted and lower-case, never shown.
    if (/^[a-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+$/.test(text)) return null;
    if (/^data-[a-z-]+$/.test(text)) return null;
    // A CSS declaration list: "display:flex;gap:8px".
    if (/^[a-z-]+\s*:\s*[^:;]+(?:;\s*[a-z-]+\s*:\s*[^:;]+)*;?$/.test(text)) return null;
    // A selector without spaces: ".card>.body".
    if (/[.>~]/.test(text) && !/[A-Z\s]/.test(text) && /^[\s\w.#>+~*,[\]():=-]+$/.test(text)) return null;
    // A JSON fragment or object literal.
    if (/^[{[]/.test(text)) return null;
    const tokens = text.split(/\s+/);
    if (tokens.length === 1) {
        // A lower-case identifier, event name or enum member, not a label.
        if (/^[a-z][a-z0-9_-]*$/.test(text)) return null;
        // A kebab-case name, including one that starts with a digit: "2-digit".
        if (/^[a-z0-9]+(?:-[a-z0-9]+)+$/.test(text)) return null;
        // camelCase and PascalCase single tokens are identifiers: a one-word
        // label is written the way a word is ("Download", "Live").
        if (/[A-Z]/.test(text.slice(1))) return null;
        // A number with a unit: "1s", "10.5%", "1500ms".
        if (/^[-+]?[\d.,:%/]+\s*[a-z%]*$/i.test(text)) return null;
        return text;
    }
    // A CSS utility list: every token lower-case, at least one hyphenated.
    if (tokens.every((token) => /^[a-z][a-z0-9-]*$/.test(token)) && tokens.some((token) => token.includes("-"))) {
        return null;
    }
    return text;
}

function countLetters(text) {
    return (text.match(LETTERS) ?? []).length;
}

/** `${…}` and markup out, entities in, whitespace collapsed. */
function normalize(raw) {
    let text = String(raw ?? "");
    text = text.replace(PLACEHOLDER, " ").replace(TAG, " ");
    for (const [entity, character] of ENTITIES) text = text.split(entity).join(character);
    return text.replace(/\s+/g, " ").trim();
}

function lineIndex(text) {
    const offsets = [0];
    for (let index = 0; index < text.length; index += 1) {
        if (text[index] === "\n") offsets.push(index + 1);
    }
    return (index) => {
        let low = 0;
        let high = offsets.length - 1;
        while (low < high) {
            const middle = Math.ceil((low + high) / 2);
            if (offsets[middle] <= index) low = middle;
            else high = middle - 1;
        }
        return low + 1;
    };
}

function readMarker(comment, line) {
    if (!MARKER.test(comment)) return null;
    const literals = [];
    for (const match of comment.matchAll(MARKER_LITERAL)) literals.push(match[1] ?? match[2]);
    const reason = comment
        .replace(MARKER, " ")
        .replace(MARKER_LITERAL, " ")
        .replace(/^[\s—:–-]+/, "")
        .replace(/[\s—:–-]+$/, "")
        .trim();
    return {literals, reason, line};
}

const REGEX_PRECEDERS = new Set([
    "", "(", ",", "=", ":", "[", "!", "&", "|", "?", "{", "}", ";", "<", ">",
    "+", "-", "*", "%", "~", "^", "return", "typeof", "instanceof", "in", "of",
    "new", "delete", "void", "case", "do", "else", "yield", "await", "throw",
]);

/**
 * Every string and template literal of a JS source, with comments read for
 * `translatable: false` markers.
 *
 * A scanner rather than a regex: a literal inside a comment, a regex or a nested
 * template expression has to be told apart from a real one, and a regex would
 * also split `"it's"` in a way that invents a second literal.
 * @param {string} text
 * @returns {{literals: {value: string, line: number}[], markers: {literals: string[], reason: string, line: number}[]}}
 */
export function scanLiterals(text) {    const lineOf = lineIndex(text);
    const literals = [];
    const markers = [];

    function addLiteral(index, raw) {
        literals.push({value: raw, line: lineOf(index)});
    }

    function addComment(index, comment) {
        const marker = readMarker(comment, lineOf(index));
        if (marker) markers.push(marker);
    }

    function readQuoted(start, quote) {
        let index = start + 1;
        let value = "";
        while (index < text.length) {
            const character = text[index];
            if (character === "\\") {
                value += text[index + 1] ?? "";
                index += 2;
                continue;
            }
            if (character === quote) return {value, end: index + 1};
            if (character === "\n") return {value, end: index};
            value += character;
            index += 1;
        }
        return {value, end: index};
    }

    function skipRegex(start) {
        let index = start + 1;
        let inClass = false;
        while (index < text.length) {
            const character = text[index];
            if (character === "\\") {
                index += 2;
                continue;
            }
            if (character === "\n") return index;
            if (character === "[") inClass = true;
            else if (character === "]") inClass = false;
            else if (character === "/" && !inClass) {
                index += 1;
                while (index < text.length && /[a-z]/i.test(text[index])) index += 1;
                return index;
            }
            index += 1;
        }
        return index;
    }

    function readTemplate(start) {
        let index = start + 1;
        let value = "";
        while (index < text.length) {
            const character = text[index];
            if (character === "\\") {
                value += text[index + 1] ?? "";
                index += 2;
                continue;
            }
            if (character === "`") return {value, end: index + 1};
            if (character === "$" && text[index + 1] === "{") {
                index = walk(index + 2, true);
                value += " ";
                continue;
            }
            value += character;
            index += 1;
        }
        return {value, end: index};
    }

    /** Walks code from `from`; with `stopAtBrace`, stops after the unmatched `}`. */
    function walk(from, stopAtBrace) {
        let index = from;
        let depth = 0;
        let previous = "";
        while (index < text.length) {
            const character = text[index];
            if (/\s/.test(character)) {
                index += 1;
                continue;
            }
            if (character === "/" && text[index + 1] === "/") {
                const end = text.indexOf("\n", index);
                const stop = end === -1 ? text.length : end;
                addComment(index, text.slice(index + 2, stop));
                index = stop;
                continue;
            }
            if (character === "/" && text[index + 1] === "*") {
                const end = text.indexOf("*/", index + 2);
                const stop = end === -1 ? text.length : end + 2;
                addComment(index, text.slice(index + 2, end === -1 ? text.length : end));
                index = stop;
                continue;
            }
            if (character === '"' || character === "'") {
                const {value, end} = readQuoted(index, character);
                addLiteral(index, value);
                index = end;
                previous = "string";
                continue;
            }
            if (character === "`") {
                const {value, end} = readTemplate(index);
                addLiteral(index, value);
                index = end;
                previous = "string";
                continue;
            }
            if (character === "/" && REGEX_PRECEDERS.has(previous)) {
                index = skipRegex(index);
                previous = "regex";
                continue;
            }
            if (stopAtBrace && character === "{") {
                depth += 1;
                index += 1;
                previous = "{";
                continue;
            }
            if (stopAtBrace && character === "}") {
                if (depth === 0) return index + 1;
                depth -= 1;
                index += 1;
                previous = "}";
                continue;
            }
            if (/[A-Za-z_$]/.test(character)) {
                let end = index + 1;
                while (end < text.length && /[\w$]/.test(text[end])) end += 1;
                previous = text.slice(index, end);
                index = end;
                continue;
            }
            previous = character;
            index += 1;
        }
        return text.length;
    }

    walk(0, false);
    return {literals, markers};
}

/**
 * The translatable-attribute values of the tags a JS literal builds.
 *
 * An attribute's text is invisible to the literal heuristic: `<i title="No ping
 * response">` normalizes to nothing once the tag is stripped, so English help
 * that only ever appears inside JS-built markup could never be a candidate and
 * never needed a marker. Reading the attributes out of the literal closes that
 * class; a value that is only a `${…}` interpolation normalizes to nothing and
 * is not counted.
 * @param {string} value Literal value; template interpolations are already spaces.
 * @returns {string[]} Normalized attribute values, in source order.
 */
function attributeTexts(value) {
    const texts = [];
    for (const tag of value.matchAll(START_OR_END_TAG)) {
        const attributes = parseAttributes(tag[3]);
        for (const attribute of TRANSLATABLE_ATTRIBUTES) {
            const raw = attributes.get(attribute);
            if (raw === undefined) continue;
            const normalized = normalize(raw);
            if (normalized) texts.push(normalized);
        }
    }
    return texts;
}

/**
 * The translatable candidates of a JS source: `scanLiterals` filtered by the
 * §1.3 heuristic, so a key, a path or an identifier is not counted as text,
 * plus the translatable attributes of any markup the literals build.
 * `texts` is every literal the file contains, before the filter: a
 * `translatable: false` marker may name plumbing the filter already dropped.
 * @param {string} text
 * @returns {{texts: Set<string>, candidates: {text: string, line: number}[], markers: {literals: string[], reason: string, line: number}[]}}
 */
export function scanCode(text) {
    const {literals, markers} = scanLiterals(text);
    const candidates = [];
    const texts = new Set();
    for (const literal of literals) {
        const normalized = normalize(literal.value);
        if (normalized) {
            texts.add(normalized);
            const candidate = classifyText(normalized);
            if (candidate !== null) candidates.push({text: candidate, line: literal.line});
        }
        for (const attributeText of attributeTexts(literal.value)) {
            texts.add(attributeText);
            const candidate = classifyText(attributeText);
            if (candidate !== null) candidates.push({text: candidate, line: literal.line});
        }
    }
    return {texts, candidates, markers};
}

/**
 * The translatable text of an HTML page: text nodes after comments, `<script>`
 * and `<style>` are blanked, plus the title/aria-label/placeholder/alt values
 * that carry two letters.
 *
 * Text an element already declares through `data-i18n` (or one of its attribute
 * variants) is the inline English fallback the applier overwrites, not a
 * candidate: counting it would make a migrated page look unmigrated.
 * @param {string} text
 * @returns {{texts: Set<string>, candidates: {text: string, line: number}[], markers: {literals: string[], reason: string, line: number}[]}}
 */
export function scanMarkup(text) {
    const lineOf = lineIndex(text);
    const candidates = [];
    const markers = [];
    const texts = new Set();
    const stack = [];

    function addText(index, raw, keyed) {
        const normalized = normalize(raw);
        if (!normalized) return;
        texts.add(normalized);
        if (keyed) return;
        const candidate = classifyText(normalized);
        if (candidate !== null) candidates.push({text: candidate, line: lineOf(index)});
    }

    let body = text.replace(/<!--([\s\S]*?)-->/g, (whole, comment, offset) => {
        const marker = readMarker(comment, lineOf(offset));
        if (marker) markers.push(marker);
        return " ".repeat(whole.length);
    });
    body = body.replace(/<script\b[\s\S]*?<\/script\s*>/gi, (whole) => " ".repeat(whole.length));
    body = body.replace(/<style\b[\s\S]*?<\/style\s*>/gi, (whole) => " ".repeat(whole.length));

    let cursor = 0;
    for (const match of body.matchAll(START_OR_END_TAG)) {
        if (match.index > cursor) addText(cursor, body.slice(cursor, match.index), stack.at(-1)?.keyed ?? false);
        cursor = match.index + match[0].length;
        const name = match[2].toLowerCase();
        if (match[1] === "/") {
            const open = stack.map((element) => element.name).lastIndexOf(name);
            if (open !== -1) stack.length = open;
            continue;
        }
        const attributes = parseAttributes(match[3]);
        for (const attribute of TRANSLATABLE_ATTRIBUTES) {
            const value = attributes.get(attribute);
            if (value === undefined) continue;
            addText(match.index, value, attributes.has(`data-i18n-${attribute}`));
        }
        const selfClosing = VOID_ELEMENTS.has(name) || /\/\s*$/.test(match[3]);
        if (!selfClosing) stack.push({name, keyed: attributes.has("data-i18n")});
    }
    if (cursor < body.length) addText(cursor, body.slice(cursor), stack.at(-1)?.keyed ?? false);
    return {texts, candidates, markers};
}

function parseAttributes(source) {
    const attributes = new Map();
    for (const match of source.matchAll(/([:\w-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g)) {
        const name = match[1].toLowerCase();
        if (!attributes.has(name)) attributes.set(name, match[2] ?? match[3] ?? match[4] ?? "");
    }
    return attributes;
}

/**
 * Applies the file's markers: marked texts leave the candidate list, a marker
 * without a reason is a finding, and so is a marker naming a string the file no
 * longer contains (stale documentation).
 * @param {{texts: Set<string>, candidates: {text: string, line: number}[], markers: {literals: string[], reason: string, line: number}[]}} scan
 * @param {(text: string) => string} displayText
 * @returns {{candidates: {text: string, line: number}[], findings: string[]}}
 */
export function applyMarkers(scan, displayText = (text) => text) {
    const findings = [];
    const marked = new Set();
    for (const marker of scan.markers) {
        if (!marker.reason) {
            findings.push(`line ${marker.line}: "translatable: false" needs a reason ("— <why>")`);
            continue;
        }
        for (const literal of marker.literals) {
            const normalized = normalize(literal);
            if (!scan.texts.has(normalized)) {
                findings.push(
                    `line ${marker.line}: "translatable: false" names ${displayText(literal)}, which is not a string in this file`,
                );
                continue;
            }
            marked.add(normalized);
        }
    }
    return {candidates: scan.candidates.filter((candidate) => !marked.has(candidate.text)), findings};
}
