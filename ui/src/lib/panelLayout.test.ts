import { describe, expect, it } from 'vitest';

import {
  coercePanelLayout,
  defaultPanelLayout,
  moveById,
  reconcileLayout,
  reorder,
  updatePanel,
  type PanelLayout,
} from './panelLayout';

const IDS = ['cpu', 'memory', 'disks', 'network', 'gpu'];

function stored(...entries: Array<Partial<PanelLayout> & { id: string }>): PanelLayout[] {
  return entries.map((entry) => ({ ...defaultPanelLayout(entry.id), ...entry }));
}

describe('coercePanelLayout', () => {
  it('treats anything that is not an array as no stored layout', () => {
    expect(coercePanelLayout(undefined)).toEqual([]);
    expect(coercePanelLayout(null)).toEqual([]);
    expect(coercePanelLayout('cpu,memory')).toEqual([]);
    expect(coercePanelLayout({ cpu: 12 })).toEqual([]);
  });

  it('drops entries with no usable id', () => {
    expect(coercePanelLayout([{ span: 6 }, { id: '', span: 6 }, { id: 'cpu' }])).toEqual([
      defaultPanelLayout('cpu'),
    ]);
  });

  it('replaces an illegal span or height with the default rather than the whole entry', () => {
    expect(coercePanelLayout([{ id: 'cpu', span: 5, height: 'enormous', hidden: 'yes' }])).toEqual([
      { id: 'cpu', span: 12, height: 'normal', hidden: false },
    ]);
  });

  it('keeps legal values untouched', () => {
    const entry = { id: 'gpu', span: 4, height: 'tall', hidden: true };
    expect(coercePanelLayout([entry])).toEqual([entry]);
  });
});

describe('reconcileLayout', () => {
  it('keeps the stored order', () => {
    const result = reconcileLayout(stored({ id: 'gpu' }, { id: 'cpu' }), ['cpu', 'gpu']);
    expect(result.map((panel) => panel.id)).toEqual(['gpu', 'cpu']);
  });

  it('appends a section the stored layout has never heard of', () => {
    // The case that matters: a release adds a panel, and someone with a saved
    // layout must still see it.
    const result = reconcileLayout(stored({ id: 'gpu' }, { id: 'cpu' }), IDS);
    expect(result.map((panel) => panel.id)).toEqual(['gpu', 'cpu', 'memory', 'disks', 'network']);
  });

  it('gives an appended section the default size, not the previous one', () => {
    const result = reconcileLayout(stored({ id: 'cpu', span: 4 }), ['cpu', 'memory']);
    expect(result[1]).toEqual(defaultPanelLayout('memory'));
  });

  it('drops a stored id that no longer names a section', () => {
    const result = reconcileLayout(stored({ id: 'ports' }, { id: 'cpu' }), ['cpu']);
    expect(result.map((panel) => panel.id)).toEqual(['cpu']);
  });

  it('keeps only the first of a duplicated id, so a panel cannot render twice', () => {
    const result = reconcileLayout(stored({ id: 'cpu', span: 4 }, { id: 'cpu', span: 8 }), ['cpu']);
    expect(result).toHaveLength(1);
    expect(result[0]?.span).toBe(4);
  });

  it('returns the canonical order when nothing is stored', () => {
    expect(reconcileLayout([], IDS).map((panel) => panel.id)).toEqual(IDS);
  });

  it('repairs an illegal span that reached it unchecked', () => {
    const rogue = [
      { id: 'cpu', span: 7, height: 'normal', hidden: false },
    ] as unknown as PanelLayout[];
    expect(reconcileLayout(rogue, ['cpu'])[0]?.span).toBe(12);
  });
});

describe('reorder', () => {
  it('moves an item forwards', () => {
    expect(reorder(['a', 'b', 'c', 'd'], 0, 2)).toEqual(['b', 'c', 'a', 'd']);
  });

  it('moves an item backwards', () => {
    expect(reorder(['a', 'b', 'c', 'd'], 3, 1)).toEqual(['a', 'd', 'b', 'c']);
  });

  it('is a no-op when the destination is where the item already is', () => {
    expect(reorder(['a', 'b', 'c'], 1, 1)).toEqual(['a', 'b', 'c']);
  });

  it('leaves the list alone rather than corrupting it on an out-of-range index', () => {
    expect(reorder(['a', 'b'], -1, 1)).toEqual(['a', 'b']);
    expect(reorder(['a', 'b'], 0, 9)).toEqual(['a', 'b']);
  });

  it('does not mutate its input', () => {
    const original = ['a', 'b', 'c'];
    reorder(original, 0, 2);
    expect(original).toEqual(['a', 'b', 'c']);
  });
});

describe('moveById', () => {
  it('moves a panel up', () => {
    const result = moveById(stored({ id: 'a' }, { id: 'b' }, { id: 'c' }), 'c', -1);
    expect(result.map((panel) => panel.id)).toEqual(['a', 'c', 'b']);
  });

  it('refuses to move the first panel up, rather than wrapping it to the end', () => {
    const list = stored({ id: 'a' }, { id: 'b' });
    expect(moveById(list, 'a', -1)).toEqual(list);
  });

  it('refuses to move the last panel down', () => {
    const list = stored({ id: 'a' }, { id: 'b' });
    expect(moveById(list, 'b', 1)).toEqual(list);
  });

  it('ignores an id that is not in the list', () => {
    const list = stored({ id: 'a' });
    expect(moveById(list, 'nope', 1)).toEqual(list);
  });
});

describe('updatePanel', () => {
  it('changes one panel and leaves the others alone', () => {
    const result = updatePanel(stored({ id: 'a' }, { id: 'b' }), 'b', { span: 6, hidden: true });
    expect(result[0]).toEqual(defaultPanelLayout('a'));
    expect(result[1]).toEqual({ id: 'b', span: 6, height: 'normal', hidden: true });
  });

  it('ignores an unknown id', () => {
    const list = stored({ id: 'a' });
    expect(updatePanel(list, 'b', { span: 4 })).toEqual(list);
  });
});
