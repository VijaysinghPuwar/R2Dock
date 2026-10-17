/** Inline logo mark from DESIGN 4.2. Bars use theme tokens so it follows Dark and Paper. */
export function LogoMark({ size = 20, small }: { size?: number; small?: boolean }) {
  const useSmall = small ?? size <= 24;
  if (useSmall) {
    // 3-bar variant for 16 to 24 px.
    return (
      <svg width={size} height={size} viewBox="200 260 624 504" aria-hidden="true" className="logo-mark">
        <rect x="224" y="282" width="480" height="132" rx="66" fill="var(--logo-bar)" />
        <rect x="320" y="446" width="480" height="132" rx="66" fill="var(--logo-accent)" />
        <rect x="224" y="610" width="480" height="132" rx="66" fill="var(--logo-bar)" />
      </svg>
    );
  }
  return (
    <svg width={size} height={size} viewBox="246 246 532 532" aria-hidden="true" className="logo-mark">
      <rect x="256" y="256" width="445" height="110" rx="55" fill="var(--logo-bar)" />
      <rect x="323" y="390" width="445" height="110" rx="55" fill="var(--logo-accent)" />
      <rect x="256" y="524" width="445" height="110" rx="55" fill="var(--logo-bar)" />
      <rect x="323" y="658" width="445" height="110" rx="55" fill="var(--logo-bar)" />
    </svg>
  );
}
