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
import { Navigation } from './components/Navigation';
import { setSampleInterval } from './lib/ipc';
import { samplesInWindow, usePreferences, type Preferences } from './lib/preferences';
import type { ProcessTree } from './lib/processTree';
import { useGpuDevices, useMetrics, useProcesses, useSystemDescription } from './lib/useLiveData';
import { Overview } from './pages/Overview';
import { Planned } from './pages/Planned';
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

  // Push the chosen tick rate to the sampler. The backend clamps anything it
  // cannot honour, so a stale stored preference cannot produce a busy loop.
  useEffect(() => {
    // Swallowed deliberately: if the backend is unreachable the pages above
    // already say so, and an unhandled rejection here would add nothing except
    // noise in the console.
    setSampleInterval(preferences.refreshMs).catch(() => {});
  }, [preferences.refreshMs]);

  const stacked = preferences.navigation === 'tabs';

  return (
    <div className={`flex h-full ${stacked ? 'flex-col' : 'flex-row'}`}>
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
          />
        )}
      </main>
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
        />
      );

    case 'processes':
      return <Processes tree={tree} loaded={processesLoaded} />;

    case 'settings':
      return <Settings preferences={preferences} onChange={onPreferenceChange} />;

    default: {
      const item = NAV_ITEMS.find((candidate) => candidate.route === route);
      return item === undefined ? <p role="alert">Unknown page.</p> : <Planned item={item} />;
    }
  }
}
