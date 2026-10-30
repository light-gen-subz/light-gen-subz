/** Compares release versions numerically. An equality check is not enough: if the latest
 * release is ever older than the installed build, `latest !== current` holds and a
 * downgrade would be offered as an update. */
export function isNewer(remote: string, current: string): boolean {
  const parse = (v: string) =>
    String(v)
      .replace(/^v/, "")
      .split(".")
      .map((n) => parseInt(n, 10) || 0);
  const [r, c] = [parse(remote), parse(current)];
  for (let i = 0; i < Math.max(r.length, c.length); i++) {
    if ((r[i] || 0) !== (c[i] || 0)) return (r[i] || 0) > (c[i] || 0);
  }
  return false;
}
