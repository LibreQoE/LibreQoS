// Pinning tests for the Insight trial surface (section S5a): the teaser page,
// the hosted-signup handoff, and the two outcome pages.
//
// The prose is the risk here. These screens are the only place the UI makes
// billing promises ("no charge until the trial ends", "cancel any time") and
// describes what the paid Insight service is, so the Spanish is pinned word for
// word: a harmless-looking rewording is what changes a promise, and neither the
// extractor nor the catalog contract can see one. The provider collects billing
// details at sign-up, so the wording that sent the operator to a later billing
// step is retired and guarded against here as well.
//
// Two things on the page do not pass through a catalog at all. The country
// picker labels 237 ISO 3166-1 codes with the engine's CLDR region names, and
// the circuit line composes a formatted number with a plural message. Both are
// checked from the source text, because `lts_trial.js` is a DOM entrypoint that
// cannot be imported here.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {t, tn} from "./index.js";
import {regionName} from "../region_names.mjs";
import {applyCancelCapNotice, UNLICENSED_MAPPED_CIRCUIT_LIMIT} from "../lts_cancel_notice.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const NODE_MANAGER = resolve(HERE, "../../..");
const CATALOG_DIR = resolve(NODE_MANAGER, "static2/i18n");
const STATIC_DIR = resolve(NODE_MANAGER, "static2");
const SOURCE_DIR = resolve(NODE_MANAGER, "js_build/src");

const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));

const TRIAL_PAGE = readFileSync(resolve(SOURCE_DIR, "lts_trial.js"), "utf8");
const TRIAL_MARKUP = readFileSync(resolve(STATIC_DIR, "lts_trial.html"), "utf8");
const SUCCESS_MARKUP = readFileSync(resolve(STATIC_DIR, "lts_trial_success.html"), "utf8");
const CONFIG_MARKUP = readFileSync(resolve(STATIC_DIR, "config_lts.html"), "utf8");

// The cancellation disclosure Oracle `legal-2` settled: one key, one sentence
// per locale, quoted verbatim at every placement. The number is a placeholder
// because the grouping is the locale's (en "1,000", es "1000").
const CANCEL_NOTICE = "trial.cancel_license_cap_notice";
const CANCEL_NOTICE_EN = "Cancelling your Insight trial or subscription also removes the Insight license, so the deployment is capped at {limit} mapped circuits and circuits beyond the cap are not created.";
const CANCEL_NOTICE_ES = "Al cancelar el período de prueba o la suscripción de Insight, también se elimina la licencia de Insight, por lo que la instalación queda sujeta al límite de {limit} circuitos asignados y los circuitos que superen ese límite no se crean.";

/** The minimum `index.js` and `locale.mjs` need: the catalogs and a document. */
function installBrowser(catalog, lang) {
    globalThis.window = {LQOS_I18N_EN: EN};
    if (lang !== "en") globalThis.window.LQOS_I18N = catalog;
    globalThis.localStorage = {getItem: () => null, setItem: () => {}};
    globalThis.document = {documentElement: {lang}, cookie: ""};
}

test("the trial paragraphs render the ruled Spanish in full", () => {
    installBrowser(ES, "es");
    const ruled = new Map([
        [
            "trial.hero.lead",
            "Aproveche todo el potencial de su red con analítica avanzada, informes y monitorización.",
        ],
        [
            "trial.success.detail",
            "LibreQoS está guardando su configuración de Insight. Los datos de facturación son obligatorios al registrarse, por lo que no se requiere ningún paso de facturación adicional.",
        ],
        [
            "trial.activated.billing",
            "Los datos de facturación son obligatorios al registrarse. No se le cobrará hasta que finalice el período de prueba y puede cancelar en cualquier momento.",
        ],
        [
            "trial.setup.lead",
            "LibreQoS creará una solicitud de registro temporal en Insight y después redirigirá su navegador al proceso de registro alojado.",
        ],
        [
            "trial.failed.detail",
            "Inténtelo de nuevo en unos instantes. Si el problema continúa, compruebe la configuración de red o póngase en contacto con el soporte.",
        ],
    ]);
    for (const [key, spanish] of ruled) {
        assert.equal(t(key), spanish, `${key} must render the ruled sentence`);
    }
    installBrowser(EN, "en");
    for (const key of ruled.keys()) {
        const english = t(key);
        assert.notEqual(english, key, `${key} must exist in the English catalog`);
        assert.match(english, /\.$/, `${key} must stay a complete sentence`);
        assert.ok(english.split(/\s+/).length >= 10, `${key} must stay a paragraph, not a fragment`);
    }
});

test("the billing sentences keep the promise the English makes", () => {
    installBrowser(ES, "es");
    // A translation that drops the no-charge wording, adds a charge, or weakens
    // "cancel any time" changes what the operator is told they agreed to.
    for (const key of ["trial.trial_terms", "trial.activated.billing", "trial.success.created_detail"]) {
        const spanish = t(key);
        assert.match(spanish, /período de prueba/, `${key} must name the trial period`);
        assert.match(spanish, /(no se le cobrará|no se realizará ningún cargo)/i, `${key} must keep the no-charge promise`);
        assert.match(spanish, /puede cancelar en cualquier momento/i, `${key} must keep the cancellation term`);
    }
    assert.match(t("trial.trial_terms"), /30 días/, "the trial length must survive translation");
    installBrowser(EN, "en");
    assert.match(t("trial.trial_terms"), /30-day/, "the English source states the trial length");
});

test("the trial billing copy states the terms the provider settled", () => {
    // English is the source of truth, so it is pinned as tightly as the Spanish:
    // "required at sign-up" is the fact that replaced the 2-day window, and
    // "no further billing step is required" is what makes that window
    // meaningless. A reworded English sentence changes the promise every
    // translation is derived from.
    installBrowser(EN, "en");
    assert.equal(
        t("trial.activated.billing"),
        "Billing details are required at sign-up. You will not be charged until the trial ends, and you can cancel any time.",
        "trial.activated.billing must state the sign-up requirement and keep the no-charge and cancellation terms",
    );
    assert.equal(
        t("trial.success.detail"),
        "LibreQoS is saving your Insight configuration now. Billing details are required at sign-up, so no further billing step is required.",
        "trial.success.detail must state the sign-up requirement and that no further billing step is needed",
    );
});

test("no trial message or trial page revives the retired post-signup billing step", () => {
    // The provider collects billing details at sign-up, so the deadline and the
    // email step are gone, not merely softened. These are the retired phrases
    // rather than a style rule: either one reappearing restores a claim the
    // driver confirmed is false, in a catalog value or in the inline English the
    // page ships before the catalog loads.
    const retired = ["within 2 days", "2 days", "Finish the billing step"];
    for (const [key, value] of Object.entries(EN.messages)) {
        if (!key.startsWith("trial.")) continue;
        for (const text of typeof value === "string" ? [value] : Object.values(value)) {
            for (const phrase of retired) {
                assert.equal(
                    text.includes(phrase),
                    false,
                    `${key} must not describe the retired billing step ("${phrase}")`,
                );
            }
        }
    }
    for (const page of ["lts_trial.html", "lts_trial_success.html"]) {
        const markup = readFileSync(resolve(STATIC_DIR, page), "utf8");
        for (const phrase of retired) {
            assert.equal(
                markup.includes(phrase),
                false,
                `${page} must not ship the retired billing step ("${phrase}")`,
            );
        }
    }
});

test("the terms-of-service acceptance stays English until a legal review clears Spanish", () => {
    // Confirmed legal deferral (the S5 Oracle review): the checkbox is the
    // operator's assent to the governing terms, so it stays in English. The
    // unlock is a legal review that approves the exact Spanish consent wording
    // and names the terms-of-service and privacy-policy documents, versions,
    // and links it binds. Until that decision lands, nobody "finishes" this
    // string: translating it would change what the operator is told they
    // agreed to, and its marker keeps it out of the candidate count.
    const labelStart = TRIAL_MARKUP.lastIndexOf("<label", TRIAL_MARKUP.indexOf('for="agreeTerms"'));
    const label = TRIAL_MARKUP.slice(labelStart, TRIAL_MARKUP.indexOf("</label>", labelStart));
    assert.equal(
        label.replace(/<[^>]*>/g, "").replace(/\s+/g, " ").trim(),
        "I agree to the terms of service and privacy policy *",
        "the acceptance statement must stay in English",
    );
    assert.doesNotMatch(label, /data-i18n/, "the acceptance statement must carry no catalog key");
    assert.match(
        TRIAL_MARKUP.slice(TRIAL_MARKUP.lastIndexOf("<!--", labelStart), labelStart),
        /translatable: false — "I agree to the terms of service and privacy policy \*"/,
        "the acceptance statement needs its translatable: false marker beside it",
    );
});

test("the country picker carries codes and takes its labels from CLDR", () => {
    const list = TRIAL_PAGE.slice(
        TRIAL_PAGE.indexOf("const ALLOWED_COUNTRY_CODES"),
        TRIAL_PAGE.indexOf("];", TRIAL_PAGE.indexOf("const ALLOWED_COUNTRY_CODES")),
    );
    const codes = [...list.matchAll(/'([a-z]{2})'/g)].map((match) => match[1]);
    assert.equal(codes.length, 237, "the allow-list is the ratchet: 237 countries the payment provider accepts");
    assert.equal(new Set(codes).size, codes.length, "a country code appears twice");
    for (const code of ["es", "us", "gb", "de"]) assert.ok(codes.includes(code), `the allow-list must keep ${code}`);
    assert.doesNotMatch(list, /\bname:/, "the allow-list must not carry English labels");
    assert.match(
        TRIAL_PAGE,
        /const countrySelect = \$\('#country'\);[\s\S]*regionName\(code\)/,
        "each option label must come from regionName",
    );
    assert.doesNotMatch(TRIAL_PAGE, /<option value="\$\{/, "the picker must not interpolate a hand-written label");
});

test("region names follow the active locale and degrade to the code", () => {
    installBrowser(ES, "es");
    assert.equal(regionName("de"), "Alemania");
    assert.equal(regionName("de"), regionName("DE"), "the code lookup must be case-insensitive");
    assert.equal(regionName("jp"), "Japón");
    installBrowser(EN, "en");
    assert.equal(regionName("de"), "Germany");
    assert.equal(regionName("xx"), "XX", "a code the engine has no name for must fall back to the code");
    assert.equal(regionName(""), "", "an empty selection must not invent a name");
});

test("the circuit count line composes a formatted number with a plural message", () => {
    // The header reads "Your Network: 1,234 Active Circuits" / "Su red: 1.234
    // circuitos activos": the number is formatted by the shared formatter and
    // the noun phrase is a plural message, so neither half can drift.
    assert.doesNotMatch(TRIAL_PAGE, /\.toLocale(?:String|TimeString|DateString)\(/, "the count must use i18n/format.js");
    assert.match(TRIAL_PAGE, /formatNumber\(displayCount\)/, "the count must go through formatNumber");
    for (const key of ["trial.count.active_circuits", "trial.count.configured_circuits"]) {
        assert.match(TRIAL_PAGE, new RegExp(`tn\\("${key.replaceAll(".", "\\.")}", displayCount\\)`), `${key} must be the composed noun phrase`);
    }
    assert.match(TRIAL_MARKUP, /<span id="circuitNumber"><\/span>/, "the number keeps its own element");
    assert.match(TRIAL_MARKUP, /<span id="circuitState"><\/span>/, "the noun phrase keeps its own element");
    installBrowser(ES, "es");
    assert.equal(tn("trial.count.active_circuits", 1), "circuito activo");
    assert.equal(tn("trial.count.active_circuits", 1234), "circuitos activos");
    assert.equal(tn("trial.count.active_circuits", 1_000_000), "de circuitos activos");
    installBrowser(EN, "en");
    assert.equal(tn("trial.count.active_circuits", 1), "Active Circuit");
    assert.equal(tn("trial.count.active_circuits", 1234), "Active Circuits");
});

// The cancellation disclosure. Every screen that promises "cancel any time" now
// also states what cancelling costs, and the licence screen shows the cap the
// sentence names, so the wording, the number's locale and the three placements
// are pinned here together.
test("the cancellation notice carries the ruled sentence and one placeholder in both locales", () => {
    // Oracle `legal-2` settled this sentence in both locales: it states that
    // cancellation removes the licence and that the deployment then falls under
    // the mapped-circuit cap, without claiming traffic is interrupted. A
    // reworded clause changes what the operator is told they are agreeing to.
    assert.equal(EN.messages[CANCEL_NOTICE], CANCEL_NOTICE_EN, "en.json must carry the settled sentence");
    assert.equal(ES.messages[CANCEL_NOTICE], CANCEL_NOTICE_ES, "es.json must carry the settled sentence");
    for (const [tag, text] of [["en", CANCEL_NOTICE_EN], ["es", CANCEL_NOTICE_ES]]) {
        const names = [...new Set([...text.matchAll(/\{([A-Za-z0-9_]+)\}/g)].map((match) => match[1]))];
        assert.deepEqual(names, ["limit"], `${tag} must interpolate the cap and nothing else`);
    }
});

test("the notice formats the cap with the active locale instead of a grouped literal", () => {
    // The catalogs carry `{limit}`, never "1,000": English groups a four-digit
    // number and Spanish CLDR does not, and a hand-written separator would make
    // that a copy decision in every locale that is added later.
    assert.equal(UNLICENSED_MAPPED_CIRCUIT_LIMIT, 1000, "the notice quotes the cap cancellation restores");
    installBrowser(EN, "en");
    assert.match(t(CANCEL_NOTICE, {limit: UNLICENSED_MAPPED_CIRCUIT_LIMIT}), /capped at 1,000 mapped circuits/);
    installBrowser(ES, "es");
    const spanish = t(CANCEL_NOTICE, {limit: UNLICENSED_MAPPED_CIRCUIT_LIMIT});
    assert.match(spanish, /límite de 1000 circuitos asignados/, "Spanish leaves a four-digit number ungrouped");
    assert.equal(/1[.,]000/.test(spanish), false, "the Spanish copy must not carry the English grouping");
    assert.equal(spanish.includes("{limit}"), false, "the placeholder must be replaced");
});

test("the notice is filled into every marked element, and nowhere else", () => {
    // The elements ship empty, so the one thing that can go wrong silently is
    // the fill: an unmarked paragraph would stay blank above the fold. The root
    // is stubbed because the contract reads the real pages as text.
    installBrowser(ES, "es");
    const hero = {textContent: null};
    const signup = {textContent: null};
    const marked = [hero, signup];
    const root = {querySelectorAll: (selector) => (selector === "[data-cancel-cap-notice]" ? marked : [])};
    assert.equal(applyCancelCapNotice(root), 2);
    assert.equal(hero.textContent, t(CANCEL_NOTICE, {limit: UNLICENSED_MAPPED_CIRCUIT_LIMIT}));
    assert.equal(signup.textContent, hero.textContent, "both placements carry the same sentence");
    assert.equal(applyCancelCapNotice({querySelectorAll: () => []}), 0, "a page without the notice is untouched");
});

test("the notice sits beside both trial terms, the billing promise, and the cap it names", () => {
    // A promise and its consequence have to be read together, so each placement
    // is asserted as "directly under the sentence it qualifies".
    const underTerms = [...TRIAL_MARKUP.matchAll(/data-i18n="trial\.trial_terms"[^>]*>\s*[^<]+\s*<\/[a-z0-9]+>\s*<p class="[^"]*" data-cancel-cap-notice>/g)];
    assert.equal(underTerms.length, 2, "the trial page states the terms twice, so the notice sits under both");

    assert.match(
        SUCCESS_MARKUP,
        /data-i18n="trial\.activated\.billing"[^>]*>[^<]+<\/p>\s*<p class="[^"]*" data-cancel-cap-notice>/,
        "the post-signup page must put the notice under the billing promise",
    );

    const capAt = CONFIG_MARKUP.indexOf('id="mappedCircuitLimit"');
    const noticeAt = CONFIG_MARKUP.indexOf("data-cancel-cap-notice");
    const capabilitiesAt = CONFIG_MARKUP.indexOf('id="licenseCapabilityBadges"');
    assert.ok(capAt !== -1, "the licence page must show the mapped-circuit cap");
    assert.ok(
        noticeAt > capAt && noticeAt < capabilitiesAt,
        "the licence page must put the notice with the cap it names",
    );

    // The sentence needs the number, so no placement may carry it as `data-i18n`
    // (which renders `{limit}` verbatim) or as a second hand-written copy.
    for (const [page, markup] of [["lts_trial.html", TRIAL_MARKUP], ["lts_trial_success.html", SUCCESS_MARKUP], ["config_lts.html", CONFIG_MARKUP]]) {
        assert.equal(
            markup.includes(`data-i18n="${CANCEL_NOTICE}"`),
            false,
            `${page} must fill the notice from script, not from the DOM translation pass`,
        );
        assert.equal(markup.includes("Cancelling your Insight"), false, `${page} must not carry its own copy of the sentence`);
    }
});

test("the shell fills the notice, and the number reaches t() as a number", () => {
    // The post-signup page ships no bundle, so the shell's call is the only code
    // that runs there. It fills every page that marks the element, which is why
    // the trial and licence pages need no caller of their own.
    const noticeModule = readFileSync(resolve(SOURCE_DIR, "lts_cancel_notice.js"), "utf8");
    const shell = readFileSync(resolve(SOURCE_DIR, "template.js"), "utf8");
    assert.match(
        noticeModule,
        /t\("trial\.cancel_license_cap_notice", \{limit: UNLICENSED_MAPPED_CIRCUIT_LIMIT\}\)/,
        "the cap must reach t() as a number so Intl groups it",
    );
    assert.match(shell, /import \{applyCancelCapNotice\} from "\.\/lts_cancel_notice\.js";/);
    assert.equal(
        (shell.match(/^applyCancelCapNotice\(\);$/gm) ?? []).length,
        1,
        "the shell fills the notice exactly once, on load",
    );
});
