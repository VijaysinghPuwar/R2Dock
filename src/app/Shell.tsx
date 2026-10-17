import { useEffect } from "react";
import { Sidebar } from "../features/connections/Sidebar";
import { WelcomeScreen } from "../features/connections/WelcomeScreen";
import { ConnectDialog } from "../features/connections/ConnectDialog";
import { BucketsView } from "../features/connections/BucketsView";
import { BucketSettingsDialog } from "../features/connections/BucketSettingsDialog";
import { Explorer } from "../features/browser/Explorer";
import { RecentView } from "../features/browser/RecentView";
import { FavoritesView } from "../features/browser/FavoritesView";
import { UploadReviewDialog } from "../features/browser/UploadReviewDialog";
import { TransferDock } from "../features/transfers/TransferDock";
import { TransfersPage } from "../features/transfers/TransfersPage";
import { SettingsPage } from "../features/settings/SettingsPage";
import { PreviewModal } from "../features/preview/PreviewModal";
import { ManageDialogs } from "../features/manage/ManageDialogs";
import { useNavStore } from "./navStore";
import { useUiStore } from "./uiStore";
import { ShortcutSheet } from "./ShortcutSheet";
import { useGlobalShortcuts } from "./useGlobalShortcuts";
import { useDragDrop } from "../features/browser/useDragDrop";
import { DropOverlay } from "../features/browser/DropOverlay";
import { InterruptedBanner } from "../features/transfers/InterruptedBanner";

const NARROW = 1200;

function useWindowWidth() {
  useEffect(() => {
    let wasNarrow = window.innerWidth < NARROW;
    const onResize = () => {
      const narrow = window.innerWidth < NARROW;
      const ui = useUiStore.getState();
      if (narrow !== wasNarrow) {
        ui.setNarrow(narrow);
        ui.setSidebarCollapsed(narrow);
        wasNarrow = narrow;
      }
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
}

function MainView() {
  const view = useNavStore((s) => s.loc.view);
  switch (view) {
    case "welcome":
      return <WelcomeScreen />;
    case "buckets":
      return <BucketsView />;
    case "browser":
      return <Explorer />;
    case "transfers":
      return <TransfersPage />;
    case "settings":
      return <SettingsPage />;
    case "recent":
      return <RecentView />;
    case "favorites":
      return <FavoritesView />;
  }
}

export function Shell() {
  const collapsed = useUiStore((s) => s.sidebarCollapsed);
  const view = useNavStore((s) => s.loc.view);
  useWindowWidth();
  useGlobalShortcuts();
  useDragDrop();

  return (
    <div className={collapsed ? "app is-collapsed" : "app"}>
      <Sidebar />
      <main className="content" aria-label="Main">
        {view !== "transfers" && view !== "welcome" && <InterruptedBanner />}
        <div className="content-main">
          <MainView />
          <DropOverlay />
        </div>
        {view !== "welcome" && <TransferDock />}
      </main>
      <ConnectDialog />
      <BucketSettingsDialog />
      <ManageDialogs />
      <UploadReviewDialog />
      <PreviewModal />
      <ShortcutSheet />
    </div>
  );
}
