/**
 * The three navigation styles.
 *
 * All of them render the same {@link NAV_ITEMS} model and expose the same
 * props, so which one is on screen is purely a preference and never a
 * difference in what the app can reach.
 */

import type { NavigationStyle } from '../lib/preferences';
import { NAV_ITEMS, SETTINGS_ITEM, type NavItem, type Route } from '../routes';

/** What every navigation style needs. */
export interface NavigationProps {
  /** The page currently shown. */
  current: Route;
  /** Called when the user picks a different page. */
  onNavigate: (route: Route) => void;
  /** Which style to render. */
  style: NavigationStyle;
  /**
   * Whether a model is loaded right now.
   *
   * The navigation is the only thing on screen in every page, which is what
   * makes it the place for this. A loaded model holds several gigabytes and is
   * now kept until it is unloaded or osstat quits (ADR-013), so from any other
   * tab there would otherwise be nothing at all to say it is there — and a
   * multi-gigabyte allocation a monitoring utility does not mention is the one
   * thing this application must never be.
   */
  modelLoaded: boolean;
}

/** Props for one entry. */
interface ItemProps {
  /** The entry to render. */
  item: NavItem;
  /** Whether it is the current page. */
  active: boolean;
  /** Whether to show the label beside the icon. */
  showLabel: boolean;
  /** Whether to mark this entry as holding a loaded model. */
  loaded: boolean;
  /** Called when picked. */
  onSelect: () => void;
}

/** What the marker is called, in the one place all three styles read it from. */
const LOADED_LABEL = 'A model is loaded';

/**
 * The mark on the Chat entry while a model is held.
 *
 * A dot rather than a word: the rail style is fourteen pixels wide with no room
 * for text, and a marker that only appeared in two of the three styles would
 * make the styles differ in what they can tell you rather than in how they look.
 * It is not decoration, so it is not `aria-hidden` — the accessible name is what
 * carries it to a screen reader, where a coloured dot says nothing.
 */
function LoadedMark(): React.JSX.Element {
  return (
    <span
      role="img"
      aria-label={LOADED_LABEL}
      title={LOADED_LABEL}
      className="size-1.5 shrink-0 rounded-full bg-accent"
    />
  );
}

/**
 * Renders one navigation entry.
 *
 * Unbuilt pages are dimmed but remain reachable. Disabling them would hide what
 * the app is going to be, and their placeholder pages say something useful.
 */
function Item({ item, active, showLabel, loaded, onSelect }: ItemProps): React.JSX.Element {
  const unbuilt = item.milestone !== undefined;

  return (
    <button
      type="button"
      onClick={onSelect}
      aria-current={active ? 'page' : undefined}
      title={unbuilt ? `${item.label} — arrives in ${item.milestone}` : item.label}
      className={[
        'flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm transition-colors',
        showLabel ? 'w-full' : 'justify-center',
        active
          ? 'bg-white/[0.07] text-text shadow-[inset_2px_0_0_var(--color-accent)]'
          : 'text-text-muted hover:bg-white/[0.04] hover:text-text',
        unbuilt && !active ? 'opacity-45' : '',
      ].join(' ')}
    >
      <span aria-hidden="true" className="w-3.5 text-center">
        {item.icon}
      </span>
      {showLabel && <span className="truncate">{item.label}</span>}
      {loaded && <LoadedMark />}
      {showLabel && unbuilt && (
        <span className="ml-auto rounded-full border border-edge px-1.5 font-mono text-[10px] text-text-muted">
          {item.milestone}
        </span>
      )}
    </button>
  );
}

/** A vertical navigation, with or without labels. */
function Rail({
  current,
  onNavigate,
  modelLoaded,
  showLabels,
}: NavigationProps & { showLabels: boolean }): React.JSX.Element {
  return (
    <nav
      aria-label="Main"
      className={`flex shrink-0 flex-col gap-0.5 border-r border-edge bg-black/15 p-2 ${
        showLabels ? 'w-44' : 'w-14'
      }`}
    >
      {showLabels && (
        <span className="px-2.5 pb-2 pt-1 text-[10px] font-semibold uppercase tracking-widest text-text-faint">
          osstat
        </span>
      )}

      {NAV_ITEMS.map((item) => (
        <Item
          key={item.route}
          item={item}
          active={item.route === current}
          showLabel={showLabels}
          loaded={modelLoaded && item.route === 'chat'}
          onSelect={() => {
            onNavigate(item.route);
          }}
        />
      ))}

      <div className="my-2 border-t border-edge" />

      <Item
        item={SETTINGS_ITEM}
        active={current === SETTINGS_ITEM.route}
        showLabel={showLabels}
        loaded={false}
        onSelect={() => {
          onNavigate(SETTINGS_ITEM.route);
        }}
      />
    </nav>
  );
}

/** A horizontal tab bar. */
function Tabs({ current, onNavigate, modelLoaded }: NavigationProps): React.JSX.Element {
  const items = [...NAV_ITEMS, SETTINGS_ITEM];

  return (
    <nav aria-label="Main" className="flex gap-1 border-b border-edge bg-black/15 px-3">
      {items.map((item) => {
        const active = item.route === current;
        const unbuilt = item.milestone !== undefined;

        return (
          <button
            key={item.route}
            type="button"
            onClick={() => {
              onNavigate(item.route);
            }}
            aria-current={active ? 'page' : undefined}
            title={unbuilt ? `${item.label} — arrives in ${item.milestone}` : item.label}
            className={[
              '-mb-px border-b-2 px-3 py-2 text-sm transition-colors',
              active
                ? 'border-accent text-text'
                : 'border-transparent text-text-muted hover:text-text',
              'flex items-center gap-2',
              unbuilt && !active ? 'opacity-45' : '',
            ].join(' ')}
          >
            {item.label}
            {modelLoaded && item.route === 'chat' && <LoadedMark />}
          </button>
        );
      })}
    </nav>
  );
}

/**
 * Renders the navigation in the requested style.
 *
 * @param props The current page, the navigate callback and the style.
 */
export function Navigation(props: NavigationProps): React.JSX.Element {
  switch (props.style) {
    case 'tabs':
      return <Tabs {...props} />;
    case 'rail':
      return <Rail {...props} showLabels={false} />;
    case 'sidebar':
      return <Rail {...props} showLabels />;
  }
}
