export function percentageChange(current: number, previous: number | null) {
  if (previous === null || previous <= 0) return null;
  return Math.round(((current - previous) / previous) * 100);
}
