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
}

/** Props for one entry. */
interface ItemProps {
  /** The entry to render. */
  item: NavItem;
  /** Whether it is the current page. */
  active: boolean;
  /** Whether to show the label beside the icon. */
  showLabel: boolean;
  /** Called when picked. */
  onSelect: () => void;
}

/**
 * Renders one navigation entry.
 *
 * Unbuilt pages are dimmed but remain reachable. Disabling them would hide what
 * the app is going to be, and their placeholder pages say something useful.
 */
function Item({ item, active, showLabel, onSelect }: ItemProps): React.JSX.Element {
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
          ? 'bg-white/[0.07] text-neutral-50 shadow-[inset_2px_0_0_var(--color-accent)]'
          : 'text-neutral-400 hover:bg-white/[0.04] hover:text-neutral-200',
        unbuilt && !active ? 'opacity-45' : '',
      ].join(' ')}
    >
      <span aria-hidden="true" className="w-3.5 text-center">
        {item.icon}
      </span>
      {showLabel && <span className="truncate">{item.label}</span>}
      {showLabel && unbuilt && (
        <span className="ml-auto rounded-full border border-edge px-1.5 font-mono text-[10px] text-neutral-500">
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
        <span className="px-2.5 pb-2 pt-1 text-[10px] font-semibold uppercase tracking-widest text-neutral-600">
          osstat
        </span>
      )}

      {NAV_ITEMS.map((item) => (
        <Item
          key={item.route}
          item={item}
          active={item.route === current}
          showLabel={showLabels}
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
        onSelect={() => {
          onNavigate(SETTINGS_ITEM.route);
        }}
      />
    </nav>
  );
}

/** A horizontal tab bar. */
function Tabs({ current, onNavigate }: NavigationProps): React.JSX.Element {
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
                ? 'border-accent text-neutral-50'
                : 'border-transparent text-neutral-400 hover:text-neutral-200',
              unbuilt && !active ? 'opacity-45' : '',
            ].join(' ')}
          >
            {item.label}
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
