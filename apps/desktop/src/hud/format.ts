/** The dock's elapsed clock. It was shared with the default HUD, which the fork deleted. */
export function formatElapsed(elapsedMs: number): string {
  const total = Math.floor(elapsedMs / 1_000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}
