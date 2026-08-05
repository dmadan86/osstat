/**
 * The icon set.
 *
 * Drawn here rather than pulled from a package. Sixteen glyphs do not justify a
 * dependency, and a library arrives with its own optical size and stroke weight
 * to reconcile against an interface whose labels are 12–14px — reconciling that
 * is the whole job, so there is nothing left for the package to do.
 *
 * Every icon is one 24×24 grid, `currentColor`, one stroke weight, round caps
 * and joins. Drawing them from `currentColor` is what makes them themeable at
 * all: an icon beside a label wears the label's colour in all four themes and
 * cannot fall out of step with it the way a fixed fill would.
 *
 * They are `aria-hidden` without exception. Each one sits beside text that
 * already says the thing, and an icon that carried meaning of its own would be
 * meaning only sighted users get.
 */

/** Every icon this set draws. */
export type IconName =
  | 'palette'
  | 'contrast'
  | 'panelLeft'
  | 'stack'
  | 'activity'
  | 'grid'
  | 'timer'
  | 'span'
  | 'cube'
  | 'chip'
  | 'folder'
  | 'sliders'
  | 'power'
  | 'window'
  | 'file'
  | 'chat'
  | 'list'
  | 'transfer';

/**
 * The drawings.
 *
 * Coordinates sit on quarter-pixel offsets rather than whole numbers because a
 * 1.75 stroke centred on an integer straddles the pixel boundary and renders
 * soft at 16px, which is the size every one of these is used at.
 */
const PATHS: Record<IconName, React.JSX.Element> = {
  palette: (
    <>
      <path d="M12 3.25c-4.83 0-8.75 3.7-8.75 8.25s3.92 8.25 8.75 8.25c.9 0 1.63-.73 1.63-1.63 0-.42-.16-.8-.43-1.09a1.6 1.6 0 0 1 1.18-2.68h1.92c2.46 0 4.45-1.99 4.45-4.45C20.75 6.2 16.83 3.25 12 3.25Z" />
      <circle cx="7.5" cy="11.5" r="1" fill="currentColor" stroke="none" />
      <circle cx="11" cy="8.25" r="1" fill="currentColor" stroke="none" />
      <circle cx="15.25" cy="9.5" r="1" fill="currentColor" stroke="none" />
    </>
  ),
  contrast: (
    <>
      <circle cx="12" cy="12" r="8.75" />
      <path d="M12 3.25a8.75 8.75 0 0 1 0 17.5Z" fill="currentColor" stroke="none" />
    </>
  ),
  panelLeft: (
    <>
      <rect x="3.25" y="4.25" width="17.5" height="15.5" rx="2.25" />
      <path d="M9.5 4.25v15.5" />
    </>
  ),
  stack: (
    <>
      <rect x="3.25" y="4" width="17.5" height="4" rx="1.5" />
      <rect x="3.25" y="10" width="17.5" height="4" rx="1.5" />
      <rect x="3.25" y="16" width="17.5" height="4" rx="1.5" />
    </>
  ),
  activity: <path d="M3.25 12h3.5l2.5 6.5 4-13 2.5 6.5h5" />,
  grid: (
    <>
      <rect x="3.25" y="3.25" width="7.5" height="7.5" rx="1.75" />
      <rect x="13.25" y="3.25" width="7.5" height="7.5" rx="1.75" />
      <rect x="3.25" y="13.25" width="7.5" height="7.5" rx="1.75" />
      <rect x="13.25" y="13.25" width="7.5" height="7.5" rx="1.75" />
    </>
  ),
  timer: (
    <>
      <circle cx="12" cy="13.5" r="7.25" />
      <path d="M12 13.5V10" />
      <path d="M9.5 2.75h5" />
      <path d="M12 2.75v3.5" />
    </>
  ),
  span: (
    <>
      <path d="M4.25 5.5v13" />
      <path d="M19.75 5.5v13" />
      <path d="M7.25 12h9.5" />
      <path d="m9.5 9.5-2.25 2.5 2.25 2.5" />
      <path d="m14.5 9.5 2.25 2.5-2.25 2.5" />
    </>
  ),
  cube: (
    <>
      <path d="M12 2.75 20.5 7.2v9.6L12 21.25 3.5 16.8V7.2Z" />
      <path d="m3.5 7.2 8.5 4.45 8.5-4.45" />
      <path d="M12 11.65v9.6" />
    </>
  ),
  chip: (
    <>
      <rect x="7.25" y="7.25" width="9.5" height="9.5" rx="1.75" />
      <path d="M10 3.25v4M14 3.25v4M10 16.75v4M14 16.75v4" />
      <path d="M3.25 10h4M3.25 14h4M16.75 10h4M16.75 14h4" />
    </>
  ),
  folder: (
    <path d="M3.25 6.75a2 2 0 0 1 2-2h3.6a2 2 0 0 1 1.6.8l1.05 1.4h7.25a2 2 0 0 1 2 2v8.3a2 2 0 0 1-2 2H5.25a2 2 0 0 1-2-2Z" />
  ),
  sliders: (
    <>
      <path d="M3.5 8h8.75M16.75 8h3.75" />
      <path d="M3.5 16h3.75M11.75 16h8.75" />
      <circle cx="14.5" cy="8" r="2.25" />
      <circle cx="9.5" cy="16" r="2.25" />
    </>
  ),
  power: (
    <>
      <path d="M12 3.25v8.5" />
      <path d="M7.4 6.6a8 8 0 1 0 9.2 0" />
    </>
  ),
  window: (
    <>
      <rect x="3.25" y="4.25" width="17.5" height="15.5" rx="2.25" />
      <path d="M3.25 9.25h17.5" />
    </>
  ),
  file: (
    <>
      <path d="M14 3.25H7.25a2 2 0 0 0-2 2v13.5a2 2 0 0 0 2 2h9.5a2 2 0 0 0 2-2V8Z" />
      <path d="M14 3.25V8h4.75" />
      <path d="M8.75 13h6.5M8.75 16.5h4" />
    </>
  ),
  chat: (
    <path d="M20.75 12.75a7 7 0 0 1-7 7H8.5l-4.25 2.5.9-3.6a7 7 0 0 1-1.9-4.8V11a7 7 0 0 1 7-7h3.5a7 7 0 0 1 7 7Z" />
  ),
  list: (
    <>
      <path d="M8.75 6.5h12M8.75 12h12M8.75 17.5h12" />
      {/* Round caps turn a zero-length segment into a dot, so the bullets are
          the same stroke as the rules beside them rather than a second shape. */}
      <path d="M4.25 6.5h.01M4.25 12h.01M4.25 17.5h.01" />
    </>
  ),
  transfer: (
    <>
      <path d="M3.75 9h13.5" />
      <path d="m14 5.75 3.25 3.25L14 12.25" />
      <path d="M20.25 15H6.75" />
      <path d="m10 11.75-3.25 3.25L10 18.25" />
    </>
  ),
};

/** What an icon needs. */
export interface IconProps {
  /** Which drawing to render. */
  name: IconName;
  /** Sizing and colour. Defaults to 16px in the surrounding text colour. */
  className?: string;
}

/**
 * Renders one icon.
 *
 * @param props The name of the drawing, and optional sizing.
 */
export function Icon({ name, className = 'size-4' }: IconProps): React.JSX.Element {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      className={`shrink-0 ${className}`}
    >
      {PATHS[name]}
    </svg>
  );
}
