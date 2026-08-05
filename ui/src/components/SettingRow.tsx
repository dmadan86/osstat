/**
 * The shape every settings row wears.
 *
 * Four files draw rows onto the settings card — `Settings.tsx` and the three
 * sections large enough to own a file — and before this they each spelled the
 * padding and the border out for themselves. They had already drifted: two used
 * `p-3` and one `px-4 py-3`, which is invisible until they sit in the same tab
 * and the labels stop lining up.
 *
 * The icon column is what makes the card scannable now that a tab holds three
 * or four rows rather than eleven: the eye lands on a shape before it reads a
 * word. It is deliberately a column and not a badge — the icons align down one
 * edge, so scanning the card costs one downward sweep rather than a hunt.
 */

import { Icon, type IconName } from './Icon';

/**
 * The padding and rule every row shares.
 *
 * `last:border-b-0` rather than `border-t` on each row: the rule belongs
 * between rows, and a top border on the first row draws a line immediately
 * under the card's own edge.
 */
export const ROW_CLASS = 'border-b border-edge px-4 py-3.5 last:border-b-0';

/** What a row's heading needs. */
export interface SettingHeaderProps {
  /** The drawing in the icon column. */
  icon: IconName;
  /** What the setting is called. */
  label: string;
  /**
   * A control belonging on the heading line itself, at the right.
   *
   * For a switch, whose whole state is one glance — anything with options
   * belongs under the description, where it has the width to show them.
   */
  control?: React.ReactNode;
  /** What the setting does, and anything the user should know before changing it. */
  children?: React.ReactNode;
}

/**
 * Renders a row's icon, name and description.
 *
 * @param props The icon, the label, an optional inline control and the description.
 */
export function SettingHeader({
  icon,
  label,
  control,
  children,
}: SettingHeaderProps): React.JSX.Element {
  return (
    <div className="flex items-start gap-3">
      <Icon name={icon} className="mt-0.5 size-4 text-text-muted" />

      <div className="min-w-0 flex-1">
        <p className="text-sm">{label}</p>
        {children !== undefined && <p className="mt-0.5 text-xs text-text-muted">{children}</p>}
      </div>

      {control}
    </div>
  );
}
