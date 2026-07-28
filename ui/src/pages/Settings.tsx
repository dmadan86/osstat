/**
 * The view settings.
 *
 * Navigation style and page layout are separate controls because they answer
 * separate questions. Presenting them as three named presets would have quietly
 * ruled out combinations — top tabs with sub-tabs, an icon rail with one long
 * page — that nothing actually prevents.
 */

import { CHOICES, type Preferences } from '../lib/preferences';

/** What the Settings page needs. */
export interface SettingsProps {
  /** The current preferences. */
  preferences: Preferences;
  /** Applies a change. */
  onChange: (update: Partial<Preferences>) => void;
}

/** One setting, rendered as a row of choices. */
function Choice<K extends keyof Preferences>({
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
        {CHOICES[setting].map((choice) => {
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
      </div>
    </div>
  );
}
