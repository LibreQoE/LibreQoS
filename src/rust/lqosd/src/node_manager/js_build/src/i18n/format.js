// Intl formatters for the node_manager UI. Every formatter reads the active
// locale from `locale()`, through the Intl tag guard, so pages never build
// "3s ago" strings by hand and a junk stored locale cannot throw mid-render.

import {intlTag} from "./intl.js";
import {locale} from "./locale.mjs";

const DEFAULT_DATE_OPTIONS = {dateStyle: "medium"};
const DEFAULT_DATE_TIME_OPTIONS = {dateStyle: "medium", timeStyle: "short"};
const DEFAULT_TIME_OPTIONS = {timeStyle: "short"};

/**
 * One decimal place, both bounds. A readout that carried a tenth keeps it —
 * the tile does not gain or lose a digit — while the decimal mark and the
 * grouping come from the locale.
 */
export const ONE_DECIMAL = {minimumFractionDigits: 1, maximumFractionDigits: 1};

// Intl constructors cost 10–35 µs and a table formats thousands of values per
// render, so one instance per locale and option set is reused.
const formatters = new Map();

function cachedIntl(kind, tag, options, create) {
    const key = `${kind}:${tag}:${JSON.stringify(options)}`;
    let formatter = formatters.get(key);
    if (formatter === undefined) {
        formatter = create();
        formatters.set(key, formatter);
    }
    return formatter;
}

/**
 * Locale-aware relative time, e.g. "3 days ago" / "hace 3 días".
 * @param {number} value Offset from now in `unit`; negative is the past.
 * @param {string} unit "second", "minute", "hour", "day", "week", "month", "year".
 * @param {Intl.RelativeTimeFormatOptions} [options]
 * @returns {string}
 */
export function relativeTime(value, unit, options) {
    const tag = intlTag(locale());
    return cachedIntl("relative", tag, options, () => new Intl.RelativeTimeFormat(tag, options)).format(value, unit);
}

/**
 * Locale-aware number formatting: group separators, decimal mark.
 * @param {number} value
 * @param {Intl.NumberFormatOptions} [options]
 * @returns {string}
 */
export function formatNumber(value, options) {
    const tag = intlTag(locale());
    return cachedIntl("number", tag, options, () => new Intl.NumberFormat(tag, options)).format(value);
}

/**
 * Plural-rules selector for the active locale; the cardinal category of a count.
 * @returns {Intl.PluralRules}
 */
export function pluralRules() {
    const tag = intlTag(locale());
    return cachedIntl("plural", tag, undefined, () => new Intl.PluralRules(tag));
}

/**
 * Date without a time of day.
 * @param {Date|number} value Date or epoch milliseconds.
 * @param {Intl.DateTimeFormatOptions} [options]
 * @returns {string}
 */
export function formatDate(value, options = DEFAULT_DATE_OPTIONS) {
    const tag = intlTag(locale());
    return cachedIntl("date", tag, options, () => new Intl.DateTimeFormat(tag, options)).format(value);
}

/**
 * Date with a short time of day.
 * @param {Date|number} value Date or epoch milliseconds.
 * @param {Intl.DateTimeFormatOptions} [options]
 * @returns {string}
 */
export function formatDateTime(value, options = DEFAULT_DATE_TIME_OPTIONS) {
    const tag = intlTag(locale());
    return cachedIntl("dateTime", tag, options, () => new Intl.DateTimeFormat(tag, options)).format(value);
}

/**
 * Time of day without a date, e.g. "2:05 PM" / "14:05".
 * @param {Date|number} value Date or epoch milliseconds.
 * @param {Intl.DateTimeFormatOptions} [options]
 * @returns {string}
 */
export function formatTime(value, options = DEFAULT_TIME_OPTIONS) {
    const tag = intlTag(locale());
    return cachedIntl("time", tag, options, () => new Intl.DateTimeFormat(tag, options)).format(value);
}
