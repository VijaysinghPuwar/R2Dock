import "./components.css";

export function Spinner({ size = 14, label }: { size?: 14 | 20; label?: string }) {
  return (
    <span className="spinner-wrap" role={label ? "status" : undefined}>
      <svg className="spinner" width={size} height={size} viewBox="0 0 20 20" aria-hidden="true">
        <circle cx="10" cy="10" r="8" fill="none" strokeWidth="1.5" className="spinner-track" />
        <path d="M10 2 a8 8 0 0 1 8 8" fill="none" strokeWidth="1.5" strokeLinecap="round" className="spinner-arc" />
      </svg>
      {label && <span className="visually-hidden">{label}</span>}
    </span>
  );
}
