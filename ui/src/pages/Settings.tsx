/**
 * The view settings.
 *
 * Navigation style and page layout are separate controls because they answer
 * separate questions. Presenting them as three named presets would have quietly
 * ruled out combinations — top tabs with sub-tabs, an icon rail with one long
 * page — that nothing actually prevents.
 */

import { useEffect, useState } from 'react';

import { InferenceRuntime } from '../components/InferenceRuntime';
import { Logging } from '../components/Logging';
import { ModelFolder } from '../components/ModelFolder';
import { isAutostartEnabled, setAutostart } from '../lib/ipc';
import { CHOICES, type Preferences } from '../lib/preferences';
import { reconcileLayout, updatePanel } from '../lib/panelLayout';
import { THEMES, type Theme } from '../lib/theme';
import { OVERVIEW_PANELS, OVERVIEW_PANEL_IDS } from './overviewPanels';

/**
 * The theme picker.
 *
 * Rendered here rather than through `Choice` because a theme is chosen by
 * looking at it. Each option carries a swatch, and the swatch is not a list of
 * hard-coded colours: it is an ordinary element wearing the same `data-theme`
 * attribute the document wears, drawing `bg-surface-raised`, `border-edge` and
 * `bg-accent` from the theme's own block in `index.css`. A preview built any
 * other way is a second copy of the palette, and the copy is what goes stale.
 */
function Themes({
  value,
  onSelect,
}: {
  value: Theme;
  onSelect: (chosen: Theme) => void;
}): React.JSX.Element {
  return (
    <div className="flex flex-col gap-2 border-b border-edge px-4 py-3 last:border-b-0">
      <div>
        <p className="text-sm">Theme</p>
        <p className="text-xs text-text-muted">
          All four are dark. osstat sits beside a terminal, and every surface, chart and meter in it
          is built for a dark background.
        </p>
      </div>

      <div role="radiogroup" aria-label="Theme" className="flex flex-col gap-1">
        {THEMES.map((theme) => {
          const selected = theme.value === value;
          return (
            <button
              key={theme.value}
              type="button"
              role="radio"
              aria-checked={selected}
              onClick={() => {
                onSelect(theme.value);
              }}
              className={`flex items-center gap-3 rounded-md border px-3 py-2 text-left transition-colors ${
                selected ? 'border-accent bg-accent/10' : 'border-edge hover:bg-white/[0.04]'
              }`}
            >
              {/* `aria-hidden`: the swatch shows what the label already says,
                  and a screen reader announcing two coloured squares per row
                  would bury the four names this control is actually made of. */}
              <span
                data-theme={theme.value}
                aria-hidden="true"
                className="flex h-7 w-10 shrink-0 items-center justify-center rounded border border-edge bg-surface-raised"
              >
                <span className="h-2.5 w-2.5 rounded-full bg-accent" />
              </span>

              <span className="min-w-0">
                <span className="block text-xs text-text">{theme.label}</span>
                <span className="block text-[11px] text-text-muted">{theme.description}</span>
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

/**
 * The start-at-sign-in switch.
 *
 * Reads and writes the operating system directly. There is deliberately no
 * stored copy: a mirror could disagree with reality the moment someone removed
 * the entry through Task Manager's Startup tab, and a switch that lies about
 * what the machine will do at sign-in is worse than no switch.
 */
function StartAtSignIn(): React.JSX.Element {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    isAutostartEnabled().then(setEnabled, (error: unknown) => {
      setProblem(error instanceof Error ? error.message : String(error));
    });
  }, []);

  const toggle = (): void => {
    const next = enabled !== true;

    setAutostart(next).then(
      () => {
        setEnabled(next);
        setProblem(null);
      },
      (error: unknown) => {
        setProblem(error instanceof Error ? error.message : String(error));
      }
    );
  };

  return (
    <div className="flex flex-col gap-2 border-b border-edge px-4 py-3 last:border-b-0">
      <div className="flex items-start justify-between gap-4">
        <div>
          <p className="text-sm">Start osstat when I sign in</p>
          <p className="text-xs text-text-muted">
            Starts in the notification area, with no window.
          </p>
        </div>

        <button
          type="button"
          role="switch"
          aria-checked={enabled === true}
          aria-label="Start osstat when I sign in"
          disabled={enabled === null && problem === null}
          onClick={toggle}
          className={`h-5 w-9 shrink-0 rounded-full border transition-colors ${
            enabled === true ? 'border-accent bg-accent/40' : 'border-edge bg-white/[0.04]'
          }`}
        >
          <span
            aria-hidden="true"
            className={`block h-3.5 w-3.5 rounded-full bg-text transition-transform ${
              enabled === true ? 'translate-x-4.5' : 'translate-x-0.5'
            }`}
          />
        </button>
      </div>

      {problem !== null && (
        <p role="alert" className="text-xs text-red-400">
          Could not read or change the sign-in entry: {problem}
        </p>
      )}
    </div>
  );
}

/** What the Settings page needs. */
export interface SettingsProps {
  /** The current preferences. */
  preferences: Preferences;
  /** Applies a change. */
  onChange: (update: Partial<Preferences>) => void;
}

/** One setting, rendered as a row of choices. */
function Choice<K extends keyof typeof CHOICES>({
  label,
  description,
  setting,
  value,
  onSelect,
}: {
  label: string;
  description: string;
  setting: K;
  value: Preferences[K];
  onSelect: (chosen: Preferences[K]) => void;
}): React.JSX.Element {
  return (
    <div className="flex flex-col gap-2 border-b border-edge px-4 py-3 last:border-b-0">
      <div>
        <p className="text-sm">{label}</p>
        <p className="text-xs text-text-muted">{description}</p>
      </div>

      <div role="radiogroup" aria-label={label} className="flex flex-wrap gap-1.5">
        {CHOICES[setting].map((choice: (typeof CHOICES)[K][number]) => {
          const selected = choice.value === value;
          return (
            <button
              key={String(choice.value)}
              type="button"
              role="radio"
              aria-checked={selected}
              onClick={() => {
                onSelect(choice.value as Preferences[K]);
              }}
              className={`rounded-md border px-3 py-1 text-xs transition-colors ${
                selected
                  ? 'border-accent bg-accent/10 text-text'
                  : 'border-edge text-text-muted hover:bg-white/[0.04] hover:text-text'
              }`}
            >
              {choice.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** The list of Overview panels, and the way back from hiding one. */
function Panels({ preferences, onChange }: SettingsProps): React.JSX.Element {
  const panels = reconcileLayout(preferences.overviewPanels, OVERVIEW_PANEL_IDS);

  return (
    <div className="flex flex-col gap-2 border-b border-edge px-4 py-3 last:border-b-0">
      <div>
        <p className="text-sm">Panels</p>
        <p className="text-xs text-text-muted">
          Which sections the Overview shows. Order and size are set from each panel&rsquo;s own
          menu.
        </p>
      </div>

      <div className="flex flex-col gap-1">
        {OVERVIEW_PANELS.map((panel) => {
          const hidden = panels.find((entry) => entry.id === panel.id)?.hidden ?? false;
          return (
            <label key={panel.id} className="flex items-center gap-2 text-xs text-text">
              <input
                type="checkbox"
                checked={!hidden}
                aria-label={panel.title}
                onChange={() => {
                  onChange({ overviewPanels: updatePanel(panels, panel.id, { hidden: !hidden }) });
                }}
                className="accent-accent"
              />
              {panel.title}
            </label>
          );
        })}
      </div>

      <div>
        <button
          type="button"
          onClick={() => {
            // Empty rather than a rebuilt default list: reconciliation fills it
            // from the sections that exist, which is the one place that knows.
            onChange({ overviewPanels: [] });
          }}
          className="rounded-md border border-edge px-3 py-1 text-xs text-text-muted hover:bg-white/[0.04] hover:text-text"
        >
          Reset Overview layout
        </button>
      </div>
    </div>
  );
}

/**
 * Renders the settings page.
 *
 * @param props The current preferences and the change handler.
 */
export function Settings({ preferences, onChange }: SettingsProps): React.JSX.Element {
  return (
    <div className="flex max-w-2xl flex-col gap-4">
      <header>
        <h2 className="text-lg font-semibold">Settings</h2>
        <p className="text-xs text-text-muted">
          Stored on this machine only. osstat makes no network request unless you ask it to —
          downloading an inference runtime, or a model from the advisor, are the only things that
          do.
        </p>
      </header>

      <div className="overflow-hidden rounded-xl border border-edge bg-surface-raised">
        <Themes
          value={preferences.theme}
          onSelect={(theme) => {
            onChange({ theme });
          }}
        />
        <Choice
          label="Navigation"
          description="Where the list of pages lives."
          setting="navigation"
          value={preferences.navigation}
          onSelect={(navigation) => {
            onChange({ navigation });
          }}
        />
        <Choice
          label="Page layout"
          description="Whether a page's sections stack on one scrolling page or become sub-tabs."
          setting="pageLayout"
          value={preferences.pageLayout}
          onSelect={(pageLayout) => {
            onChange({ pageLayout });
          }}
        />
        <Panels preferences={preferences} onChange={onChange} />
        <Choice
          label="Refresh interval"
          description="How often the machine is measured. Sampling pauses on its own while the window is minimised."
          setting="refreshMs"
          value={preferences.refreshMs}
          onSelect={(refreshMs) => {
            onChange({ refreshMs });
          }}
        />
        <Choice
          label="History window"
          description="How much past the charts show."
          setting="historySeconds"
          value={preferences.historySeconds}
          onSelect={(historySeconds) => {
            onChange({ historySeconds });
          }}
        />
        <StartAtSignIn />
        <Choice
          label="When I close the window"
          description="osstat can keep running in the notification area, where its icon brings it back."
          setting="closeBehaviour"
          value={preferences.closeBehaviour}
          onSelect={(closeBehaviour) => {
            onChange({ closeBehaviour });
          }}
        />
        <InferenceRuntime />
        <ModelFolder />
        <Logging
          level={preferences.logLevel}
          onChangeLevel={(logLevel) => {
            onChange({ logLevel });
          }}
        />
      </div>
    </div>
  );
}
