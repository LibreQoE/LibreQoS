export function TrimToFit(s, limit=20) {
    if (s.length < limit) return s;
    return s.slice(0, limit);
}