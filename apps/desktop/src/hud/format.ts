/** Shared between the default HUD and the side dock, which show the same clock. */
export function formatElapsed(elapsedMs: number): string {
  const total = Math.floor(elapsedMs / 1_000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}
