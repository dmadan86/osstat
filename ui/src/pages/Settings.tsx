/**
 * The view settings.
 *
 * Navigation style and page layout are separate controls because they answer
 * separate questions. Presenting them as three named presets would have quietly
 * ruled out combinations — top tabs with sub-tabs, an icon rail with one long
 * page — that nothing actually prevents.
 *
 * Eleven settings on one scrolling card had become a list to search rather than
 * a page to read, so they are grouped into {@link TABS}. The grouping is by the
 * question each setting answers, not by the code that implements it: "how much
 * past the charts show" sits with "how often the machine is measured" under
 * Monitoring, even though one is a chart concern and one is a sampler concern,
 * because somebody changing one has usually come to change the other.
 *
 * The tabs are `tablist`/`tab`/`tabpanel` with a roving tab stop and arrow-key
 * movement, which is the pattern a screen reader and a keyboard both already
 * know. Only the selected panel is mounted: an unmounted `InferenceRuntime` is
 * one fewer event subscription and one fewer status probe on a page somebody
 * opened to change their theme.
 */

import { useEffect, useState } from 'react';

import { Icon, type IconName } from '../components/Icon';
import { InferenceRuntime } from '../components/InferenceRuntime';
import { Logging } from '../components/Logging';
import { ModelFolder } from '../components/ModelFolder';
import { ROW_CLASS, SettingHeader } from '../components/SettingRow';
import { isAutostartEnabled, setAutostart } from '../lib/ipc';
import { CHOICES, type Preferences } from '../lib/preferences';
import { reconcileLayout, updatePanel } from '../lib/panelLayout';
import { THEMES, type Theme } from '../lib/theme';
import { OVERVIEW_PANELS, OVERVIEW_PANEL_IDS } from './overviewPanels';

/** The groups the settings are divided into, in the order they appear. */
const TABS = [
  { id: 'appearance', label: 'Appearance', icon: 'palette' },
  { id: 'monitoring', label: 'Monitoring', icon: 'activity' },
  { id: 'models', label: 'Models', icon: 'cube' },
  { id: 'system', label: 'System', icon: 'sliders' },
] as const satisfies readonly { id: string; label: string; icon: IconName }[];

/** Which group is on screen. */
type TabId = (typeof TABS)[number]['id'];

/** Which way each arrow key moves along the tab strip. */
const ARROW_STEPS: Record<string, number> = { ArrowRight: 1, ArrowLeft: -1 };

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
    <div className={ROW_CLASS}>
      <SettingHeader icon="contrast" label="Theme">
        All four are dark. osstat sits beside a terminal, and every surface, chart and meter in it
        is built for a dark background.
      </SettingHeader>

      <div role="radiogroup" aria-label="Theme" className="mt-2.5 flex flex-col gap-1">
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

  const control = (
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
  );

  return (
    <div className={ROW_CLASS}>
      <SettingHeader icon="power" label="Start osstat when I sign in" control={control}>
        Starts in the notification area, with no window.
      </SettingHeader>

      {problem !== null && (
        <p role="alert" className="mt-2.5 text-xs text-red-400">
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
  icon,
  label,
  description,
  setting,
  value,
  onSelect,
}: {
  icon: IconName;
  label: string;
  description: string;
  setting: K;
  value: Preferences[K];
  onSelect: (chosen: Preferences[K]) => void;
}): React.JSX.Element {
  return (
    <div className={ROW_CLASS}>
      <SettingHeader icon={icon} label={label}>
        {description}
      </SettingHeader>

      <div role="radiogroup" aria-label={label} className="mt-2.5 flex flex-wrap gap-1.5">
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
    <div className={ROW_CLASS}>
      <SettingHeader icon="grid" label="Panels">
        Which sections the Overview shows. Order and size are set from each panel&rsquo;s own menu.
      </SettingHeader>

      <div className="mt-2.5 flex flex-col gap-1">
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

      <div className="mt-2.5">
        <button
          type="button"
          onClick={() => {
            // Empty rather than a rebuilt default list: reconciliation fills it
            // from the sections that exist, which is the one place that knows.
            onChange({ overviewPanels: [] });
          }}
          className="rounded-md border border-edge px-3 py-1 text-xs text-text-muted transition-colors hover:bg-white/[0.04] hover:text-text"
        >
          Reset Overview layout
        </button>
      </div>
    </div>
  );
}

/** How the app looks: the theme, and where the furniture goes. */
function Appearance({ preferences, onChange }: SettingsProps): React.JSX.Element {
  return (
    <>
      <Themes
        value={preferences.theme}
        onSelect={(theme) => {
          onChange({ theme });
        }}
      />
      <Choice
        icon="panelLeft"
        label="Navigation"
        description="Where the list of pages lives."
        setting="navigation"
        value={preferences.navigation}
        onSelect={(navigation) => {
          onChange({ navigation });
        }}
      />
      <Choice
        icon="stack"
        label="Page layout"
        description="Whether a page's sections stack on one scrolling page or become sub-tabs."
        setting="pageLayout"
        value={preferences.pageLayout}
        onSelect={(pageLayout) => {
          onChange({ pageLayout });
        }}
      />
    </>
  );
}

/** What gets measured, how often, and how far back. */
function Monitoring({ preferences, onChange }: SettingsProps): React.JSX.Element {
  return (
    <>
      <Panels preferences={preferences} onChange={onChange} />
      <Choice
        icon="timer"
        label="Refresh interval"
        description="How often the machine is measured. Sampling pauses on its own while the window is minimised."
        setting="refreshMs"
        value={preferences.refreshMs}
        onSelect={(refreshMs) => {
          onChange({ refreshMs });
        }}
      />
      <Choice
        icon="span"
        label="History window"
        description="How much past the charts show."
        setting="historySeconds"
        value={preferences.historySeconds}
        onSelect={(historySeconds) => {
          onChange({ historySeconds });
        }}
      />
    </>
  );
}

/** The two things osstat downloads, and where they land. */
function Models(): React.JSX.Element {
  return (
    <>
      <InferenceRuntime />
      <ModelFolder />
    </>
  );
}

/** How osstat behaves on this machine, and what it writes down. */
function System({ preferences, onChange }: SettingsProps): React.JSX.Element {
  return (
    <>
      <StartAtSignIn />
      <Choice
        icon="window"
        label="When I close the window"
        description="osstat can keep running in the notification area, where its icon brings it back."
        setting="closeBehaviour"
        value={preferences.closeBehaviour}
        onSelect={(closeBehaviour) => {
          onChange({ closeBehaviour });
        }}
      />
      <Logging
        level={preferences.logLevel}
        onChangeLevel={(logLevel) => {
          onChange({ logLevel });
        }}
      />
    </>
  );
}

/**
 * Renders the settings page.
 *
 * @param props The current preferences and the change handler.
 */
export function Settings({ preferences, onChange }: SettingsProps): React.JSX.Element {
  const [tab, setTab] = useState<TabId>('appearance');

  // Arrow keys move the selection and the focus together, which is what a
  // `tablist` promises. The buttons are read off the strip the event came from
  // rather than held in refs — there is exactly one strip, and it is the
  // element the handler is bound to.
  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>): void => {
    const step = ARROW_STEPS[event.key];
    if (step === undefined) {
      return;
    }

    event.preventDefault();
    const index = TABS.findIndex((entry) => entry.id === tab);
    const next = (index + step + TABS.length) % TABS.length;
    const destination = TABS[next];
    if (destination === undefined) {
      return;
    }

    setTab(destination.id);
    event.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]')[next]?.focus();
  };

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

      <div
        role="tablist"
        aria-label="Settings sections"
        onKeyDown={onKeyDown}
        className="flex items-center gap-1 border-b border-edge"
      >
        {TABS.map((entry) => {
          const selected = entry.id === tab;
          return (
            <button
              key={entry.id}
              type="button"
              role="tab"
              id={`settings-tab-${entry.id}`}
              aria-selected={selected}
              aria-controls={`settings-panel-${entry.id}`}
              tabIndex={selected ? 0 : -1}
              onClick={() => {
                setTab(entry.id);
              }}
              className={[
                // Square, like the sub-tabs in `Section.tsx`: the accent rule
                // is the selection, and rounding its corners away from the
                // strip it underlines is the one thing that would break it.
                '-mb-px flex items-center gap-2 border-b-2 px-3 py-2 text-sm transition-colors',
                'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-accent',
                selected
                  ? 'border-accent text-text'
                  : 'border-transparent text-text-muted hover:text-text',
              ].join(' ')}
            >
              <Icon name={entry.icon} />
              {entry.label}
            </button>
          );
        })}
      </div>

      <div
        role="tabpanel"
        id={`settings-panel-${tab}`}
        aria-labelledby={`settings-tab-${tab}`}
        className="overflow-hidden rounded-xl border border-edge bg-surface-raised"
      >
        {tab === 'appearance' && <Appearance preferences={preferences} onChange={onChange} />}
        {tab === 'monitoring' && <Monitoring preferences={preferences} onChange={onChange} />}
        {tab === 'models' && <Models />}
        {tab === 'system' && <System preferences={preferences} onChange={onChange} />}
      </div>
    </div>
  );
}
