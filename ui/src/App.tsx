/**
 * The application shell.
 *
 * Owns the current route, the view preferences, and the live-data
 * subscriptions the pages read. Everything else is a page or a component.
 */

import { useEffect, useState } from 'react';

import type { GpuDevice } from './bindings/GpuDevice';
import type { MetricsSample } from './bindings/MetricsSample';
import type { SystemDescription } from './bindings/SystemDescription';
import { EndProcessDialog } from './components/EndProcessDialog';
import { Navigation } from './components/Navigation';
import { setCloseBehaviour, setSampleInterval } from './lib/ipc';
import { samplesInWindow, usePreferences, type Preferences } from './lib/preferences';
import type { ProcessTree } from './lib/processTree';
import type { EndProcessTarget } from './lib/termination';
import {
  useGpuDevices,
  useMetrics,
  useProcesses,
  useSystemDescription,
  useTrayHidden,
} from './lib/useLiveData';
import { Llm } from './pages/Llm';
import { Overview } from './pages/Overview';
import { Planned } from './pages/Planned';
import { Ports } from './pages/Ports';
import { Processes } from './pages/Processes';
import { Settings } from './pages/Settings';
import { NAV_ITEMS, type Route } from './routes';

/** Renders the whole application. */
export function App(): React.JSX.Element {
  const [route, setRoute] = useState<Route>('overview');
  const [preferences, updatePreferences] = usePreferences();

  const system = useSystemDescription();
  const { samples, latest } = useMetrics(
    samplesInWindow(preferences.historySeconds, preferences.refreshMs)
  );
  const { tree, loaded } = useProcesses();
  const gpus = useGpuDevices();
  const hiddenToTray = useTrayHidden();
  const [endTarget, setEndTarget] = useState<EndProcessTarget | null>(null);

  // Push the chosen tick rate to the sampler. The backend clamps anything it
  // cannot honour, so a stale stored preference cannot produce a busy loop.
  useEffect(() => {
    // Swallowed deliberately: if the backend is unreachable the pages above
    // already say so, and an unhandled rejection here would add nothing except
    // noise in the console.
    setSampleInterval(preferences.refreshMs).catch(() => {});
  }, [preferences.refreshMs]);

  useEffect(() => {
    // Replayed for the same reason as the interval: Rust holds the value it
    // needs at close time, and the front-end is where the preference lives.
    setCloseBehaviour(preferences.closeBehaviour).catch(() => {});
  }, [preferences.closeBehaviour]);

  const stacked = preferences.navigation === 'tabs';

  return (
    <div className={`flex h-full ${stacked ? 'flex-col' : 'flex-row'}`}>
      {/* `inert` while the dialog is open, rather than relying on the overlay
          alone: the overlay blocks the mouse but not Tab, and without this a
          user could tab to another row's "End" button behind the dialog and
          reach it with the keyboard. */}
      <div className="contents" inert={endTarget !== null}>
        <Navigation current={route} onNavigate={setRoute} style={preferences.navigation} />

        <main className="min-w-0 flex-1 overflow-auto px-5 py-4">
          {system.status === 'error' && (
            <p role="alert" className="text-sm text-red-400">
              Could not reach the osstat backend: {system.message}
            </p>
          )}

          {system.status === 'loading' && (
            <p role="status" className="text-sm text-neutral-400">
              Reading system information…
            </p>
          )}

          {system.status === 'ready' && (
            <Page
              route={route}
              system={system.value}
              samples={samples}
              latest={latest}
              gpus={gpus}
              tree={tree}
              processesLoaded={loaded}
              preferences={preferences}
              onPreferenceChange={updatePreferences}
              hiddenToTray={hiddenToTray}
              onEndProcess={setEndTarget}
            />
          )}
        </main>
      </div>

      {/* Rendered here rather than inside a page: it subscribes to
          processes:tick for its own reasons regardless of which page is
          visible, and a user who switches pages mid-confirmation should not
          lose it.

          Keyed on the target's identity so that switching targets — e.g.
          tabbing to a different row's "End" button while a confirmation is
          open — remounts the dialog's state machine instead of reusing it.
          Without the key, `state` (which phase the dialog is in) survives a
          `target` change, so a dialog already advanced to `force` for one
          process would carry that phase over to a newly selected one and
          skip its confirmation and critical-process check entirely. */}
      {endTarget !== null && (
        <EndProcessDialog
          key={`${endTarget.key.pid}:${endTarget.key.startedAt}`}
          target={endTarget}
          onClose={() => {
            setEndTarget(null);
          }}
        />
      )}
    </div>
  );
}

/** Everything a page might need. */
interface PageProps {
  /** Which page to show. */
  route: Route;
  /** The machine's identity. */
  system: SystemDescription;
  /** Recent samples, oldest first. */
  samples: MetricsSample[];
  /** The most recent sample. */
  latest: MetricsSample | null;
  /** GPUs found, or `null` while probing. */
  gpus: GpuDevice[] | null;
  /** The process tree. */
  tree: ProcessTree;
  /** Whether the first process snapshot has arrived. */
  processesLoaded: boolean;
  /** The current view preferences. */
  preferences: Preferences;
  /** Applies a preference change. */
  onPreferenceChange: (update: Partial<Preferences>) => void;
  /** Whether the window has been hidden to the tray this session. */
  hiddenToTray: boolean;
  /** Opens the confirmation dialog for a process the user wants ended. */
  onEndProcess: (target: EndProcessTarget) => void;
}

/** Chooses the page for the current route. */
function Page({
  route,
  system,
  samples,
  latest,
  gpus,
  tree,
  processesLoaded,
  preferences,
  onPreferenceChange,
  hiddenToTray,
  onEndProcess,
}: PageProps): React.JSX.Element {
  switch (route) {
    case 'overview':
      return (
        <Overview
          system={system}
          samples={samples}
          latest={latest}
          gpus={gpus}
          layout={preferences.pageLayout}
          panels={preferences.overviewPanels}
          onPanelsChange={(overviewPanels) => {
            onPreferenceChange({ overviewPanels });
          }}
          showTrayNotice={hiddenToTray && !preferences.hasSeenTrayNotice}
          onTrayNoticeSeen={() => {
            onPreferenceChange({ hasSeenTrayNotice: true });
          }}
        />
      );

    case 'processes':
      return <Processes tree={tree} loaded={processesLoaded} onEndProcess={onEndProcess} />;

    case 'ports':
      return <Ports tree={tree} onEndProcess={onEndProcess} />;

    case 'llm':
      return <Llm />;

    case 'settings':
      return <Settings preferences={preferences} onChange={onPreferenceChange} />;

    default: {
      const item = NAV_ITEMS.find((candidate) => candidate.route === route);
      return item === undefined ? <p role="alert">Unknown page.</p> : <Planned item={item} />;
    }
  }
}
