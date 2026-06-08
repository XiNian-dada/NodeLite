/**
 * Dotted-version comparison, ported from assets/index.html:2348. Pure.
 * Returns <0 if left < right, 0 if equal, >0 if left > right. Non-numeric
 * segments count as 0 (matching the legacy `parseInt(v,10) || 0`).
 */
export function compareVersions(left: string, right: string): number {
  const a = String(left)
    .split('.')
    .map((v) => parseInt(v, 10) || 0);
  const b = String(right)
    .split('.')
    .map((v) => parseInt(v, 10) || 0);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if ((a[i] || 0) !== (b[i] || 0)) return (a[i] || 0) - (b[i] || 0);
  }
  return 0;
}

/** Strip a leading `v`/`V` from a release tag (e.g. "v2.3.0" → "2.3.0"). */
export function normalizeVersionTag(tag: string): string {
  return String(tag).trim().replace(/^v/i, '');
}

/** True for stable release tags; prerelease/test suffixes require explicit install. */
export function isStableVersionTag(tag: string): boolean {
  return /^\d+(?:\.\d+)*(?:\+[0-9A-Za-z.-]+)?$/.test(normalizeVersionTag(tag));
}

/** True when `latest` is strictly newer than `current` (tags `v`-tolerant). */
export function isNewerVersion(latest: string, current: string): boolean {
  return compareVersions(normalizeVersionTag(latest), normalizeVersionTag(current)) > 0;
}
