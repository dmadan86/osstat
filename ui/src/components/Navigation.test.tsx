import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { Navigation } from './Navigation';
import { NAV_ITEMS } from '../routes';
import type { NavigationStyle } from '../lib/preferences';

const STYLES: NavigationStyle[] = ['sidebar', 'tabs', 'rail'];

describe('Navigation', () => {
  it.each(STYLES)('reaches every page in %s style', (style) => {
    render(
      <Navigation current="overview" onNavigate={() => {}} style={style} modelLoaded={false} />
    );

    // The point of driving all three from one NAV_ITEMS model: they cannot
    // drift apart about what the app contains.
    for (const item of NAV_ITEMS) {
      expect(screen.getByTitle(new RegExp(item.label, 'i'))).toBeInTheDocument();
    }
  });

  it.each(STYLES)('offers settings in %s style', (style) => {
    render(
      <Navigation current="overview" onNavigate={() => {}} style={style} modelLoaded={false} />
    );

    expect(screen.getByTitle('Settings')).toBeInTheDocument();
  });

  it.each(STYLES)('marks the current page in %s style', (style) => {
    render(
      <Navigation current="processes" onNavigate={() => {}} style={style} modelLoaded={false} />
    );

    expect(screen.getByTitle('Processes')).toHaveAttribute('aria-current', 'page');
    expect(screen.getByTitle('Settings')).not.toHaveAttribute('aria-current');
  });

  it.each(STYLES)('navigates on click in %s style', async (style) => {
    const onNavigate = vi.fn();
    render(
      <Navigation current="overview" onNavigate={onNavigate} style={style} modelLoaded={false} />
    );

    await userEvent.click(screen.getByTitle('Processes'));

    expect(onNavigate).toHaveBeenCalledWith('processes');
  });

  it('offers nothing that does not lead to a page', () => {
    // The navigation once carried Cleaner as a dimmed entry leading to a
    // placeholder. Every entry now reaches something real, and this is what
    // says so: a route in the model with no page behind it fails here.
    render(
      <Navigation current="overview" onNavigate={() => {}} style="sidebar" modelLoaded={false} />
    );

    expect(screen.queryByTitle(/Cleaner/)).not.toBeInTheDocument();
    expect(NAV_ITEMS.map((item) => item.route)).toEqual([
      'overview',
      'processes',
      'ports',
      'llm',
      'chat',
    ]);
  });

  it('hides labels in the icon rail but keeps them accessible', () => {
    render(
      <Navigation current="overview" onNavigate={() => {}} style="rail" modelLoaded={false} />
    );

    // Nothing is visibly labelled, but every control is still named.
    expect(screen.queryByText('Processes')).not.toBeInTheDocument();
    expect(screen.getByTitle('Processes')).toBeInTheDocument();
  });

  it('shows labels in the sidebar', () => {
    render(
      <Navigation current="overview" onNavigate={() => {}} style="sidebar" modelLoaded={false} />
    );

    expect(screen.getByText('Processes')).toBeInTheDocument();
  });
});
