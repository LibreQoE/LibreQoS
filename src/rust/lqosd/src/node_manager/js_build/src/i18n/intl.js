// Guards the tag every Intl constructor receives. `locale()` returns whatever
// the browser stored or the page declared, and Intl throws RangeError on a tag
// it cannot parse, so the tag is checked once here instead of at each call site.

import {FALLBACK_LOCALE} from "./locale.mjs";

// Anything shaped like a language tag may still be rejected by Intl; the shape
// pre-filter only keeps obviously unusable values off the probe constructor.
const LOCALE_TAG_SHAPE = /^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})*$/;
const acceptedTags = new Set();

function accepts(tag) {
    if (!LOCALE_TAG_SHAPE.test(tag)) return false;
    try {
        new Intl.DateTimeFormat(tag);
        return true;
    } catch {
        return false;
    }
}

/**
 * The tag Intl should use for `tag`: the tag itself when Intl accepts it,
 * otherwise English. Accepted tags are memoized, so a render path pays for the
 * probe once per distinct candidate. A rejected candidate is never cached,
 * because the value behind it can change.
 * @param {string} tag Candidate from `locale()`.
 * @returns {string}
 */
export function intlTag(tag) {
    if (acceptedTags.has(tag)) return tag;
    if (!accepts(tag)) return FALLBACK_LOCALE;
    acceptedTags.add(tag);
    return tag;
}
