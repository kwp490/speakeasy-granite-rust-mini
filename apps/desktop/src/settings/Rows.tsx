import { useId, type ChangeEvent, type ReactNode } from "react";

import { messages } from "../catalog";

/**
 * The three building blocks every settings page is made of (UI-GUIDE
 * "Information architecture"): a group is a small label over one card, and a
 * card holds rows. A row is a name, at most one short line under it, and its
 * control on the right.
 *
 * One pattern on purpose. The workspace used to present a setting six different
 * ways -- bordered checkbox cards, plain checkboxes, radio lists, fieldsets,
 * fact grids and model cards -- and a user could not scan any page of it.
 */
export function SettingGroup({ label, children }: { label?: string; children: ReactNode }) {
  const id = useId();
  return (
    <section aria-labelledby={label === undefined ? undefined : id} className="setting-group">
      {label !== undefined && (
        <h3 className="setting-group-label" id={id}>
          {label}
        </h3>
      )}
      <div className="setting-card">{children}</div>
    </section>
  );
}

/**
 * One row. `title` names the setting; `detail` is the one line under it, and
 * may be a status (a word with a mark) rather than prose. `control` sits on the
 * right. `labelFor` ties the title to a native control by id, so a select or an
 * input is announced by the row's name.
 */
export function SettingRow({
  title,
  detail,
  control,
  labelFor,
  titleId,
  detailId,
  children,
}: {
  title: ReactNode;
  detail?: ReactNode;
  control?: ReactNode;
  labelFor?: string;
  titleId?: string;
  detailId?: string;
  children?: ReactNode;
}) {
  return (
    <div className="setting-row">
      <div className="setting-row-main">
        <div className="setting-row-label">
          {labelFor === undefined ? (
            <span className="setting-row-title" id={titleId}>
              {title}
            </span>
          ) : (
            <label className="setting-row-title" htmlFor={labelFor} id={titleId}>
              {title}
            </label>
          )}
          {detail !== undefined && (
            <span className="setting-row-detail" id={detailId}>
              {detail}
            </span>
          )}
        </div>
        {control !== undefined && <div className="setting-row-control actions">{control}</div>}
      </div>
      {children}
    </div>
  );
}

/**
 * A row that opens to show more: a list and its editor, or exact values.
 *
 * A native `<details>`, so it is keyboard operable and announced as expandable
 * without any ARIA of ours.
 */
export function SettingExpander({
  title,
  detail,
  value,
  children,
}: {
  title: string;
  detail?: string;
  value?: ReactNode;
  children: ReactNode;
}) {
  return (
    <details className="setting-expander">
      <summary className="setting-row-main">
        <span className="setting-row-label">
          <span className="setting-row-title">{title}</span>
          {detail !== undefined && <span className="setting-row-detail">{detail}</span>}
        </span>
        <span className="setting-row-control">
          {value !== undefined && <span className="setting-row-value">{value}</span>}
          <span aria-hidden="true" className="setting-expander-chevron">
            ›
          </span>
        </span>
      </summary>
      <div className="setting-expander-body">{children}</div>
    </details>
  );
}

/**
 * An on/off switch.
 *
 * `role="switch"` on a native checkbox, so the browser still owns the state,
 * the keyboard (Space) and the change event, and a switch that the backend
 * refuses snaps back because `checked` is always the stored value. The state is
 * also written as a word beside the track: colour is never the only signal.
 */
export function Switch({
  checked,
  onChange,
  label,
  describedBy,
  disabled,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  describedBy?: string;
  disabled?: boolean;
}) {
  const toggle = (event: ChangeEvent<HTMLInputElement>) => onChange(event.target.checked);
  return (
    <label className="switch">
      <span aria-hidden="true" className="switch-word">
        {checked ? messages.switchOn : messages.switchOff}
      </span>
      <input
        aria-describedby={describedBy}
        aria-label={label}
        checked={checked}
        disabled={disabled}
        onChange={toggle}
        role="switch"
        type="checkbox"
      />
      <span aria-hidden="true" className="switch-track" />
    </label>
  );
}

/** A status word with a mark, for the line under a row's name. */
export function StatusText({
  tone,
  children,
  testId,
}: {
  tone: "ok" | "warn" | "bad" | "neutral";
  children: ReactNode;
  testId?: string;
}) {
  const mark = tone === "ok" ? "✓" : tone === "neutral" ? "" : "!";
  return (
    <span className="status-text" data-testid={testId} data-tone={tone}>
      {mark !== "" && <span aria-hidden="true">{mark} </span>}
      {children}
    </span>
  );
}
