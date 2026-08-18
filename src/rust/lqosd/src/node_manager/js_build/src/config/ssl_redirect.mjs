export const SSL_DISABLE_REDIRECT_DELAY_MS = 1500;

export function directHttpUrlFromLocation(locationLike, port = "9123") {
    const url = new URL(locationLike.href);
    url.protocol = "http:";
    url.username = "";
    url.password = "";
    url.port = port;
    url.pathname = "/";
    url.search = "";
    url.hash = "";
    return url.href;
}

export function sslDisableRedirectTarget(outcome, locationLike) {
    if (typeof outcome?.target_url === "string" && outcome.target_url.trim().length > 0) {
        return outcome.target_url;
    }
    return directHttpUrlFromLocation(locationLike);
}
