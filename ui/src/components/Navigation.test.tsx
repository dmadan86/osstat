import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { Navigation } from './Navigation';
import { NAV_ITEMS } from '../routes';
import type { NavigationStyle } from '../lib/preferences';

const STYLES: NavigationStyle[] = ['sidebar', 'tabs', 'rail'];

describe('Navigation', () => {
  it.each(STYLES)('reaches every page in %s style', (style) => {
    render(<Navigation current="overview" onNavigate={() => {}} style={style} />);

    // The point of driving all three from one NAV_ITEMS model: they cannot
    // drift apart about what the app contains.
    for (const item of NAV_ITEMS) {
      expect(screen.getByTitle(new RegExp(item.label, 'i'))).toBeInTheDocument();
    }
  });

  it.each(STYLES)('offers settings in %s style', (style) => {
    render(<Navigation current="overview" onNavigate={() => {}} style={style} />);

    expect(screen.getByTitle('Settings')).toBeInTheDocument();
  });

  it.each(STYLES)('marks the current page in %s style', (style) => {
    render(<Navigation current="processes" onNavigate={() => {}} style={style} />);

    expect(screen.getByTitle('Processes')).toHaveAttribute('aria-current', 'page');
    expect(screen.getByTitle('Settings')).not.toHaveAttribute('aria-current');
  });

  it.each(STYLES)('navigates on click in %s style', async (style) => {
    const onNavigate = vi.fn();
    render(<Navigation current="overview" onNavigate={onNavigate} style={style} />);

    await userEvent.click(screen.getByTitle('Processes'));

    expect(onNavigate).toHaveBeenCalledWith('processes');
  });

  it('keeps unbuilt pages reachable rather than disabling them', async () => {
    // Their placeholder says what is coming; a disabled button says nothing.
    const onNavigate = vi.fn();
    render(<Navigation current="overview" onNavigate={onNavigate} style="sidebar" />);

    const cleaner = screen.getByTitle(/Cleaner/);
    expect(cleaner).not.toBeDisabled();

    await userEvent.click(cleaner);
    expect(onNavigate).toHaveBeenCalledWith('cleaner');
  });

  it('names the milestone of an unbuilt page', () => {
    render(<Navigation current="overview" onNavigate={() => {}} style="sidebar" />);

    expect(screen.getByTitle('Cleaner — arrives in M3')).toBeInTheDocument();
  });

  it('hides labels in the icon rail but keeps them accessible', () => {
    render(<Navigation current="overview" onNavigate={() => {}} style="rail" />);

    // Nothing is visibly labelled, but every control is still named.
    expect(screen.queryByText('Processes')).not.toBeInTheDocument();
    expect(screen.getByTitle('Processes')).toBeInTheDocument();
  });

  it('shows labels in the sidebar', () => {
    render(<Navigation current="overview" onNavigate={() => {}} style="sidebar" />);

    expect(screen.getByText('Processes')).toBeInTheDocument();
  });
});
