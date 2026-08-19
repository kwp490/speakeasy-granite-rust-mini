import { useId, type ReactNode } from "react";

/**
 * A collapsed disclosure panel.
 *
 * Used for the Technical details panel and the Show raw values panel — both
 * places where exact package or contract identifiers belong, and
 * neither of which should be the first thing a user reads.
 *
 * A native `<details>` rather than a hand-rolled toggle: it is keyboard operable
 * and correctly announced without any ARIA of ours, and the settings window is
 * where keyboard operability is required (UI-GUIDE "Accessibility and input").
 */
export function Disclosure({
  summary,
  hint,
  children,
}: {
  summary: string;
  hint?: string;
  children: ReactNode;
}) {
  const hintId = useId();
  return (
    <details className="disclosure">
      <summary aria-describedby={hint === undefined ? undefined : hintId}>{summary}</summary>
      {hint !== undefined && (
        <p className="disclosure-hint" id={hintId}>
          {hint}
        </p>
      )}
      {children}
    </details>
  );
}
