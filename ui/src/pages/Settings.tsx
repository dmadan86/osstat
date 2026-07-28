/**
 * The view settings.
 *
 * Navigation style and page layout are separate controls because they answer
 * separate questions. Presenting them as three named presets would have quietly
 * ruled out combinations — top tabs with sub-tabs, an icon rail with one long
 * page — that nothing actually prevents.
 */

import { CHOICES, type Preferences } from '../lib/preferences';
import { reconcileLayout, updatePanel } from '../lib/panelLayout';
import { OVERVIEW_PANELS, OVERVIEW_PANEL_IDS } from './overviewPanels';

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
        <p className="text-xs text-neutral-500">{description}</p>
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
                  ? 'border-accent bg-accent/10 text-neutral-50'
                  : 'border-edge text-neutral-400 hover:bg-white/[0.04] hover:text-neutral-200'
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
        <p className="text-xs text-neutral-500">
          Which sections the Overview shows. Order and size are set from each panel&rsquo;s own
          menu.
        </p>
      </div>

      <div className="flex flex-col gap-1">
        {OVERVIEW_PANELS.map((panel) => {
          const hidden = panels.find((entry) => entry.id === panel.id)?.hidden ?? false;
          return (
            <label key={panel.id} className="flex items-center gap-2 text-xs text-neutral-300">
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
          className="rounded-md border border-edge px-3 py-1 text-xs text-neutral-400 hover:bg-white/[0.04] hover:text-neutral-200"
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
        <p className="text-xs text-neutral-500">
          Stored on this machine only. osstat makes no network requests.
        </p>
      </header>

      <div className="overflow-hidden rounded-xl border border-edge bg-surface-raised">
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
        <Choice
          label="When I close the window"
          description="osstat can keep running in the notification area, where its icon brings it back."
          setting="closeBehaviour"
          value={preferences.closeBehaviour}
          onSelect={(closeBehaviour) => {
            onChange({ closeBehaviour });
          }}
        />
      </div>
    </div>
  );
}
