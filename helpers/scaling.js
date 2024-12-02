export function trimTrailingZeros(num) {
    // Convert the number to a string with sufficient decimal places
    var str = num.toFixed(20);

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


// Scale a number to T/G/M/K, with a fixed number of decimal places.
export function scaleNumber(n, fixed=2, trimZeroes=true) {
    if (n >= 1000000000000) {
        return (n / 1000000000000).toFixed(fixed) + "T";
    } else if (n >= 1000000000) {
        return (n / 1000000000).toFixed(fixed) + "G";
    } else if (n >= 1000000) {
        return (n / 1000000).toFixed(fixed) + "M";
    } else if (n >= 1000) {
        return (n / 1000).toFixed(fixed) + "K";
    }
    if (trimZeroes) n = trimTrailingZeros(n)

    return n;
}

// Scale nanoseconds to a time period
export function scaleNanos(n, precision=2) {
    if (n === 0) return "-";
    if (n > 60000000000) {
        return (n / 60000000000).toFixed(precision) + "m";
    }else if (n > 1000000000) {
        return (n / 1000000000).toFixed(precision) + "s";
    } else if (n > 1000000) {
        return (n / 1000000).toFixed(precision) + "ms";
    } else if (n > 1000) {
        return (n / 1000).toFixed(precision) + "µs";
    }
    return n + "ns";
}