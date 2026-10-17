import { useNavStore, type SettingsSection } from "../../app/navStore";
import { ConnectionsSection } from "./ConnectionsSection";
import {
  AboutSection,
  AdvancedSection,
  AppearanceSection,
  NotificationsSection,
  SecuritySection,
  TransfersSection,
} from "./PreferenceSections";
import "./settings.css";

const SECTIONS: Array<{ id: SettingsSection; label: string }> = [
  { id: "connections", label: "Connections" },
  { id: "appearance", label: "Appearance" },
  { id: "transfers", label: "Transfers" },
  { id: "security", label: "Security" },
  { id: "notifications", label: "Notifications" },
  { id: "advanced", label: "Advanced" },
  { id: "about", label: "About" },
];

function SectionBody({ section }: { section: SettingsSection }) {
  switch (section) {
    case "connections":
      return <ConnectionsSection />;
    case "appearance":
      return <AppearanceSection />;
    case "transfers":
      return <TransfersSection />;
    case "security":
      return <SecuritySection />;
    case "notifications":
      return <NotificationsSection />;
    case "advanced":
      return <AdvancedSection />;
    case "about":
      return <AboutSection />;
  }
}

export function SettingsPage() {
  const section = useNavStore((s) => s.settingsSection);
  const setSection = useNavStore((s) => s.setSettingsSection);

  return (
    <div className="page settings-page">
      <h1 className="page-title settings-title">Settings</h1>
      <div className="settings-layout">
        <nav className="settings-nav" aria-label="Settings sections">
          <ul>
            {SECTIONS.map((s) => (
              <li key={s.id}>
                <button
                  type="button"
                  className={s.id === section ? "settings-nav-item is-active" : "settings-nav-item"}
                  aria-current={s.id === section ? "page" : undefined}
                  onClick={() => setSection(s.id)}
                >
                  {s.label}
                </button>
              </li>
            ))}
          </ul>
        </nav>
        <div className="settings-content">
          <SectionBody section={section} />
        </div>
      </div>
    </div>
  );
}
