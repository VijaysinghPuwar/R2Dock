import { Button } from "../../components/Button";
import { LogoMark } from "../../components/Logo";
import { useAppInfoValue } from "../../app/queries";
import { useUiStore } from "../../app/uiStore";
import "./connections.css";

export function WelcomeScreen() {
  const openConnect = useUiStore((s) => s.openConnect);
  const info = useAppInfoValue();
  return (
    <div className="welcome">
      <div className="welcome-column">
        <LogoMark size={48} />
        <h1 className="welcome-title">Your R2 storage, on your desktop.</h1>
        <p className="welcome-sub">Keys stay on this computer. No sign-up.</p>
        <Button variant="primary" size="lg" onClick={() => openConnect({ mode: "new", showHelp: false })}>
          Connect R2 account
        </Button>
        <button type="button" className="link-button" onClick={() => openConnect({ mode: "new", showHelp: true })}>
          Where do I find my keys?
        </button>
      </div>
      {info.version && <p className="welcome-version">Version {info.version}</p>}
    </div>
  );
}
