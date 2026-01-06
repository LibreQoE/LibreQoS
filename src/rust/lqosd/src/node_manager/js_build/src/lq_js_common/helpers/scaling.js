export function trimTrailingZeros(str) {
    // Check if the string contains a decimal point
    if (str.indexOf('.') === -1) {
        // No decimal point, return the string as is
        return str;
    } else {
        // Remove trailing zeros after the decimal point
        str = str.replace(/(\.\d*?[1-9])0+$/g, '$1');
        // Remove the decimal point if there are no digits after it
        str = str.replace(/\.0+$/g, '');
        return str;
    }
}

export function toNumber(value, fallback = 0) {
    if (value === null || value === undefined) {
        return fallback;
    }
    if (typeof value === "bigint") {
        try {
            const num = Number(value);
            return Number.isFinite(num) ? num : fallback;
        } catch (err) {
            return fallback;
        }
    }
    if (typeof value === "number") {
        return Number.isFinite(value) ? value : fallback;
    }
    try {
        const num = Number(value);
        return Number.isFinite(num) ? num : fallback;
    } catch (err) {
        return fallback;
    }
}

function toFixedDigits(value, fallback = 2, min = 0, max = 20) {
    let digits = Math.round(toNumber(value, fallback));
    if (!Number.isFinite(digits)) digits = fallback;
    digits = Math.max(min, Math.min(max, digits));
    return digits;
}


// Scale a number to T/G/M/K, with a fixed number of decimal places.
export function scaleNumber(n, fixed=2) {
    fixed = toFixedDigits(fixed, 2);
    n = toNumber(n, 0);
    if (n >= 1000000000000) {
        return trimTrailingZeros((n / 1000000000000).toFixed(fixed)) + "T";
    } else if (n >= 1000000000) {
        return trimTrailingZeros((n / 1000000000).toFixed(fixed)) + "G";
    } else if (n >= 1000000) {
        return trimTrailingZeros((n / 1000000).toFixed(fixed)) + "M";
    } else if (n >= 1000) {
        return trimTrailingZeros((n / 1000).toFixed(fixed)) + "K";
    }

    return n;
}

// Scale nanoseconds to a time period
export function scaleNanos(n, precision=2) {
    precision = toFixedDigits(precision, 2);
    n = toNumber(n, 0);
    if (n === 0) return "-";
    if (n > 60000000000) {
        return trimTrailingZeros((n / 60000000000).toFixed(precision)) + "m";
    }else if (n > 1000000000) {
        return trimTrailingZeros((n / 1000000000).toFixed(precision)) + "s";
    } else if (n > 1000000) {
        return trimTrailingZeros((n / 1000000).toFixed(precision)) + "ms";
    } else if (n > 1000) {
        return trimTrailingZeros((n / 1000).toFixed(precision)) + "µs";
    }
    return n + "ns";
}
