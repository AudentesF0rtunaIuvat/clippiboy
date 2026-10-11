import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { useEngine } from "@/store";
import { Card } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import type { Route } from "./NavBar";

/** How long the notice has to be in front of someone before it counts as read. */
const SHOWN_MS = 10_000;
const TICK_MS = 500;

/**
 * Someone who updated never saw the setup and its usage stats switch: tell them
 * once, at the top of the page. A toast was too fleeting — the window often
 * comes up behind the game or the browser, and it was gone before anyone
 * looked. So the clock only runs while the window has the focus, and no ping
 * goes out before this has been seen (`usageStatsNoticed`).
 */
export function UsageNotice({ onNavigate }: { onNavigate: (route: Route) => void }) {
  const { config, patchConfig, setSettingsTab } = useEngine(
    useShallow((s) => ({
      config: s.config,
      patchConfig: s.patchConfig,
      setSettingsTab: s.setSettingsTab,
    })),
  );
  const due = config.setupDone && config.usageStats && !config.usageStatsNoticed;
  const [shownMs, setShownMs] = useState(0);

  useEffect(() => {
    if (!due) return;
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible" && document.hasFocus()) {
        setShownMs((ms) => ms + TICK_MS);
      }
    }, TICK_MS);
    return () => window.clearInterval(timer);
  }, [due]);

  useEffect(() => {
    if (due && shownMs >= SHOWN_MS) void patchConfig({ usageStatsNoticed: true });
  }, [due, shownMs, patchConfig]);

  if (!due) return null;

  const seen = () => void patchConfig({ usageStatsNoticed: true });
  const settings = () => {
    seen();
    setSettingsTab("behaviour");
    onNavigate("settings");
  };

  return (
    <Card className="mb-6 flex items-center gap-4 border-accent/40 bg-accent/10 p-4">
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium text-ink">ClippiBoy now sends anonymous usage stats</p>
        <p className="mt-0.5 text-xs text-ink-muted">
          Once a day: the version, how many clips, which features. No content, no account. You can
          turn it off in Settings → Behaviour.
        </p>
      </div>
      <Button size="sm" variant="ghost" onClick={settings}>
        Settings
      </Button>
      <Button size="sm" variant="primary" onClick={seen}>
        Got it
      </Button>
    </Card>
  );
}
