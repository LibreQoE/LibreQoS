export function listenOnceMatchingWithTimeout(wsClient, eventName, timeoutMs, matches, handler, onTimeout) {
    let done = false;
    const cleanup = wsClient.on(eventName, (msg) => {
        if (done) return;
        if (!matches(msg)) return;
        done = true;
        clearTimeout(timer);
        cleanup();
        handler(msg);
    });
    const timer = setTimeout(() => {
        if (done) return;
        done = true;
        cleanup();
        onTimeout();
    }, timeoutMs);
    return () => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        cleanup();
    };
}
