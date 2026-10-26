// Number formatting for figures. Every figure is mono; these only fix the decimals.

export function fmtInt(n: number | null | undefined): string {
  if (n === null || n === undefined || Number.isNaN(n)) return "—";
  return Math.round(n).toLocaleString("en-US");
}

export function fmt1(n: number | null | undefined): string {
  if (n === null || n === undefined || Number.isNaN(n)) return "—";
  return n.toFixed(1);
}

export function fmtCelsius(n: number | null | undefined): string {
  if (n === null || n === undefined) return "no reading";
  return `${n.toFixed(1)} °C`;
}

export function fmtSeconds(ms: number): string {
  return `${Math.round(ms / 1000)} s`;
}

export function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n));
}
