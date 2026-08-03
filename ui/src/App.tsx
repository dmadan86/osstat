/**
 * The application shell.
 *
 * Owns the current route, the view preferences, and the live-data
 * subscriptions the pages read. Everything else is a page or a component.
 */

import { useEffect, useRef, useState } from 'react';

import type { GpuDevice } from './bindings/GpuDevice';
import type { MetricsSample } from './bindings/MetricsSample';
import type { ModelSession } from './bindings/ModelSession';
import type { SystemDescription } from './bindings/SystemDescription';
import { EndProcessDialog } from './components/EndProcessDialog';
import { Navigation } from './components/Navigation';
import { chatStatus, logUiEvent, setCloseBehaviour, setSampleInterval } from './lib/ipc';
import { samplesInWindow, usePreferences, type Preferences } from './lib/preferences';
import type { ProcessTree } from './lib/processTree';
import type { EndProcessTarget } from './lib/termination';
import { applyTheme } from './lib/theme';
import {
  useGpuDevices,
  useMetrics,
  useProcesses,
  useSystemDescription,
  useTrayHidden,
} from './lib/useLiveData';
import { Chat } from './pages/Chat';
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
  const [openedModel, setOpenedModel] = useState<ModelSession | null>(null);

  // What is loaded when the app opens, which is normally nothing: a session is
  // reaped at startup if a previous run left one. Asked anyway, because the
  // shell is now the thing that remembers a model across pages and starting
  // that memory from a guess rather than from Rust is how the marker ends up
  // describing a server that is not there.
  useEffect(() => {
    let cancelled = false;

    chatStatus().then(
      (open) => {
        if (!cancelled) setOpenedModel(open);
      },
      () => {
        // A status that cannot be read leaves the marker off. Drawing one for a
        // session nobody could confirm would be the wrong way round.
      }
    );

    return () => {
      cancelled = true;
    };
  }, []);

  /**
   * Moves to another page, keeping any open session.
   *
   * **This used to drop it**, because leaving the chat page ended the server.
   * ADR-013 recorded that and has been reversed: the model now survives moving
   * between tabs, so the shell holds on to what is open and hands it straight
   * back on return. That is what makes coming back instant rather than a frame
   * of the no-model form over a model that is loaded — and it is what the
   * navigation's marker is drawn from while the user is somewhere else.
   */
  function navigate(next: Route): void {
    setRoute(next);
  }

  // The theme is already on `<html>` when this first runs -- `theme-boot.js`
  // put it there before the first paint, which is the only way to open on the
  // chosen theme rather than flashing the default one. This effect exists for
  // the *change*: it is what makes picking a theme in Settings take effect.
  // Re-asserting the boot script's work on mount costs one attribute write and
  // means the document and React can never hold different ideas of the theme.
  useEffect(() => {
    applyTheme(preferences.theme);
  }, [preferences.theme]);

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

  // The front end reports into Rust's log rather than keeping one of its own,
  // so a session reads as one ordered story instead of two files somebody has
  // to interleave by timestamp. Which page, never what is on it.
  const mounted = useRef(false);
  useEffect(() => {
    logUiEvent(mounted.current ? 'pageChanged' : 'ready').catch(() => {});
    mounted.current = true;
  }, [route]);

  /** Applies a preference change, and notes that one happened. */
  function changePreference(update: Partial<Preferences>): void {
    updatePreferences(update);
    logUiEvent('settingChanged').catch(() => {});
  }

  const stacked = preferences.navigation === 'tabs';

  return (
    <div className={`flex h-full ${stacked ? 'flex-col' : 'flex-row'}`}>
      {/* `inert` while the dialog is open, rather than relying on the overlay
          alone: the overlay blocks the mouse but not Tab, and without this a
          user could tab to another row's "End" button behind the dialog and
          reach it with the keyboard. */}
      <div className="contents" inert={endTarget !== null}>
        <Navigation
          current={route}
          onNavigate={navigate}
          style={preferences.navigation}
          modelLoaded={openedModel !== null}
        />

        <main className="min-w-0 flex-1 overflow-auto px-5 py-4">
          {system.status === 'error' && (
            <p role="alert" className="text-sm text-red-400">
              Could not reach the osstat backend: {system.message}
            </p>
          )}

          {system.status === 'loading' && (
            <p role="status" className="text-sm text-text-muted">
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
              onPreferenceChange={changePreference}
              hiddenToTray={hiddenToTray}
              onEndProcess={setEndTarget}
              openedModel={openedModel}
              onSessionChange={setOpenedModel}
              onModelOpened={(session) => {
                setOpenedModel(session);
                setRoute('chat');
              }}
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
  /** The session that is open, wherever it was opened from. */
  openedModel: ModelSession | null;
  /** Records what the chat page opened, unloaded or adopted. */
  onSessionChange: (session: ModelSession | null) => void;
  /** Takes the user to the chat with the session Run just opened. */
  onModelOpened: (session: ModelSession) => void;
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
  openedModel,
  onSessionChange,
  onModelOpened,
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
      return (
        <Llm
          onModelOpened={onModelOpened}
          openedModel={openedModel}
          onSessionChange={onSessionChange}
        />
      );

    case 'chat':
      return <Chat opened={openedModel} onSessionChange={onSessionChange} />;

    case 'settings':
      return <Settings preferences={preferences} onChange={onPreferenceChange} />;

    default: {
      const item = NAV_ITEMS.find((candidate) => candidate.route === route);
      return item === undefined ? <p role="alert">Unknown page.</p> : <Planned item={item} />;
    }
  }
}
