import type { ReactNode } from "react";

export function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <section className="settings-section" aria-labelledby={`settings-${id}`}>
      <h2 id={`settings-${id}`} className="settings-heading">
        {title}
      </h2>
      <div className="settings-group">{children}</div>
    </section>
  );
}

export function ControlRow({ label, helper, children }: { label: string; helper?: string; children: ReactNode }) {
  return (
    <div className="setting-row">
      <div className="setting-text">
        <span>{label}</span>
        {helper && <span className="small muted">{helper}</span>}
      </div>
      <div className="setting-control">{children}</div>
    </div>
  );
}
