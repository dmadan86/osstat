import { describe, expect, it } from 'vitest';

import {
  memoryAreaOption,
  perCoreHeatmapOption,
  percentAreaOption,
  throughputOption,
  type TimePoint,
} from './options';
import { diskStatus, SERIES } from './palette';

/** A few seconds of a series. */
function points(values: number[]): TimePoint[] {
  return values.map((value, index) => ({ at: 1_700_000_000_000 + index * 2000, value }));
}

/** Reads the series array off an option object. */
function seriesOf(option: { series?: unknown }): Record<string, unknown>[] {
  return (option.series ?? []) as Record<string, unknown>[];
}

describe('percentAreaOption', () => {
  it('pins the axis to 100 rather than fitting it to the data', () => {
    // Auto-scaling would draw a dramatic mountain range out of an idle machine.
    const option = percentAreaOption(points([2, 3, 2.5]), 'CPU');

    expect(option.yAxis).toMatchObject({ min: 0, max: 100 });
  });

  it('plots one point per sample', () => {
    const option = percentAreaOption(points([1, 2, 3, 4]), 'CPU');

    expect(seriesOf(option)[0]?.data).toEqual([1, 2, 3, 4]);
  });

  it('uses the first categorical slot, not the UI accent', () => {
    // The UI accent #00b3f2 fails the lightness band for a data mark.
    const option = percentAreaOption(points([1]), 'CPU');
    const line = seriesOf(option)[0]?.lineStyle as { color: string };

    expect(line.color).toBe(SERIES[0]);
    expect(line.color).not.toBe('#00b3f2');
  });

  it('carries no legend, because one series needs none', () => {
    expect(percentAreaOption(points([1]), 'CPU').legend).toBeUndefined();
  });

  it('produces a valid option for an empty series', () => {
    const option = percentAreaOption([], 'CPU');

    expect(seriesOf(option)[0]?.data).toEqual([]);
  });
});

describe('memoryAreaOption', () => {
  it('scales the axis to installed memory when it is known', () => {
    const option = memoryAreaOption(points([1000]), 8192);

    expect(option.yAxis).toMatchObject({ max: 8192 });
  });

  it('omits the ceiling rather than setting it to zero when total is unknown', () => {
    const option = memoryAreaOption(points([1000]), 0);

    expect((option.yAxis as { max?: number }).max).toBeUndefined();
  });
});

describe('throughputOption', () => {
  it('puts both directions on one shared axis', () => {
    // Two y-scales would let the two series be scaled independently and invite
    // a comparison between them that is not true.
    const option = throughputOption(points([10, 20]), points([1, 2]));

    expect(Array.isArray(option.yAxis)).toBe(false);
    expect(seriesOf(option)).toHaveLength(2);
  });

  it('assigns categorical slots in fixed order', () => {
    const option = throughputOption(points([1]), points([2]));
    const [down, up] = seriesOf(option);

    expect((down?.lineStyle as { color: string }).color).toBe(SERIES[0]);
    expect((up?.lineStyle as { color: string }).color).toBe(SERIES[1]);
  });

  it('names both series so identity is never carried by colour alone', () => {
    const option = throughputOption(points([1]), points([2]));

    expect(seriesOf(option).map((series) => series.name)).toEqual(['Down', 'Up']);
  });
});

describe('perCoreHeatmapOption', () => {
  it('uses one sequential ramp rather than a colour per core', () => {
    // Sixteen cores is not sixteen series; no sixteen hues are distinguishable.
    const option = perCoreHeatmapOption(Array.from({ length: 16 }, () => 50));
    const visualMap = option.visualMap as { inRange: { color: string[] } };

    expect(visualMap.inRange.color.length).toBeGreaterThan(1);
    expect(seriesOf(option)).toHaveLength(1);
  });

  it('places one cell per core', () => {
    const option = perCoreHeatmapOption([1, 2, 3, 4, 5]);

    expect((seriesOf(option)[0]?.data as unknown[]).length).toBe(5);
  });

  it('wraps onto a second row past sixteen cores', () => {
    const option = perCoreHeatmapOption(Array.from({ length: 24 }, () => 10));
    const data = seriesOf(option)[0]?.data as [number, number, number][];

    expect(data[0]).toEqual([0, 0, 10]);
    expect(data[16]).toEqual([0, 1, 10]);
  });

  it('scales the ramp over the full percentage range, not the observed one', () => {
    const option = perCoreHeatmapOption([3, 4, 5]);

    expect(option.visualMap).toMatchObject({ min: 0, max: 100 });
  });

  it('runs dark-to-bright, so an idle core recedes instead of glaring', () => {
    // The ramp's low end must approach the surface it is drawn on, and this
    // surface is dark. Reversed, the brightest cell would be the idle one.
    const option = perCoreHeatmapOption([0, 100]);
    const ramp = (option.visualMap as { inRange: { color: string[] } }).inRange.color;

    expect(ramp[0]).toBe('#184f95');
    expect(ramp.at(-1)).toBe('#cde2fb');
  });

  it('leaves a gap between cells so adjacent cores stay countable', () => {
    const option = perCoreHeatmapOption([1, 2]);
    const style = seriesOf(option)[0]?.itemStyle as { borderWidth: number };

    expect(style.borderWidth).toBeGreaterThan(0);
  });

  it('produces a valid option for a machine reporting no cores', () => {
    expect(() => perCoreHeatmapOption([])).not.toThrow();
  });
});

describe('diskStatus', () => {
  it('says nothing about a volume with room left', () => {
    expect(diskStatus(0.5)).toBeNull();
    expect(diskStatus(0.89)).toBeNull();
  });

  it('flags a nearly full volume', () => {
    expect(diskStatus(0.91)).toBe('serious');
  });

  it('escalates a volume that is effectively out of space', () => {
    expect(diskStatus(0.99)).toBe('critical');
  });
});
