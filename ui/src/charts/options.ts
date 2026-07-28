/**
 * Chart options, built by pure functions.
 *
 * Nothing here touches the DOM or ECharts itself, which is what makes the
 * charts testable: a test asserts on the option object rather than trying to
 * read a canvas.
 *
 * The form of each chart was chosen before its colour, from the job the data
 * has to do:
 *
 * - CPU and GPU over time — one series, a trend → area line, one hue, no legend
 *   (the section heading names it).
 * - Per-core utilisation — magnitude across a grid → **heatmap on a sequential
 *   ramp**. Sixteen cores is not sixteen series; no set of sixteen hues is
 *   distinguishable, and the attempt fails every accessibility check.
 * - Memory — part-to-whole over time → stacked area, two series.
 * - Network — two distinct series in the same unit → two lines on **one** axis.
 *   A second y-axis would let the two be scaled independently and silently
 *   invite a comparison that is not true.
 */

import type { EChartsOption } from './echarts';
import { AREA_OPACITY, INK, SEQUENTIAL_ON_DARK, SERIES } from './palette';
import { formatBytes, formatClock, formatPercent, formatRate } from '../lib/format';

/** A point on a time axis. */
export interface TimePoint {
  /** Milliseconds since the Unix epoch. */
  at: number;
  /** The measured value. */
  value: number;
}

/** One row of an axis-triggered tooltip, as ECharts hands it to a formatter. */
interface TooltipRow {
  /** The x value under the pointer. On a time axis, epoch milliseconds. */
  axisValue: number;
  /** The series this row describes. */
  seriesName: string;
  /** The datum, as the `[time, value]` pair the series was given. */
  value: [number, number];
  /** The coloured swatch ECharts pre-renders for this series. */
  marker: string;
}

/**
 * Shared axis, grid and tooltip chrome.
 *
 * The tooltip is formatted here rather than per series because a `time` axis
 * heads each tooltip with the axis value, and ECharts' default rendering of
 * that is a full ISO-style timestamp. Passing the value formatter in keeps one
 * tooltip shape across the charts while each still reads in its own unit.
 *
 * @param formatValue Renders a y value in the chart's unit.
 */
function base(formatValue: (value: number) => string): EChartsOption {
  return {
    animation: false,
    grid: { left: 48, right: 12, top: 12, bottom: 22, containLabel: false },
    tooltip: {
      trigger: 'axis',
      backgroundColor: INK.surface,
      borderColor: INK.axis,
      textStyle: { color: INK.primary, fontSize: 11 },
      axisPointer: { type: 'line', lineStyle: { color: INK.muted, width: 1 } },
      formatter: (params: unknown) => {
        const rows = (Array.isArray(params) ? params : [params]) as TooltipRow[];
        const first = rows[0];
        if (first === undefined) return '';

        const body = rows
          .map((row) => `${row.marker}${row.seriesName} ${formatValue(Number(row.value[1]))}`)
          .join('<br/>');

        return `${formatClock(Number(first.axisValue))}<br/>${body}`;
      },
    },
  };
}

/**
 * An axis of real time, labelled with local clock times.
 *
 * `type: 'time'` rather than `'category'`: samples are not evenly spaced. The
 * sampler slows to 5 s while the window is hidden and stops entirely while it
 * is paused, and a category axis would draw both of those as though no time had
 * passed.
 */
function timeAxis(): NonNullable<EChartsOption['xAxis']> {
  return {
    type: 'time',
    axisLine: { lineStyle: { color: INK.axis } },
    axisTick: { show: false },
    axisLabel: {
      color: INK.muted,
      fontSize: 10,
      hideOverlap: true,
      formatter: (value: number) => formatClock(value),
    },
  };
}

/**
 * A single-series area chart over time, scaled `0..100`.
 *
 * The axis is pinned to 100 rather than fitted to the data: an idle machine
 * whose axis auto-scaled to 3% would draw a dramatic mountain range out of
 * nothing at all.
 *
 * @param points The series to draw.
 * @param name What the series measures, shown in the tooltip.
 */
export function percentAreaOption(points: readonly TimePoint[], name: string): EChartsOption {
  return {
    ...base(formatPercent),
    xAxis: timeAxis(),
    yAxis: {
      type: 'value',
      min: 0,
      max: 100,
      splitLine: { lineStyle: { color: INK.gridline } },
      axisLabel: { color: INK.muted, fontSize: 10, formatter: '{value}%' },
    },
    series: [
      {
        name,
        type: 'line',
        showSymbol: false,
        smooth: false,
        lineStyle: { width: 2, color: SERIES[0] },
        areaStyle: { color: SERIES[0], opacity: AREA_OPACITY },
        data: points.map((point) => [point.at, point.value]),
      },
    ],
  };
}

/**
 * A stacked area chart of memory in use against total.
 *
 * @param used Bytes resident, over time.
 * @param total Total physical memory, for the axis ceiling.
 */
export function memoryAreaOption(used: readonly TimePoint[], total: number): EChartsOption {
  return {
    ...base(formatBytes),
    xAxis: timeAxis(),
    yAxis: {
      type: 'value',
      min: 0,
      // Pinned to installed memory where it is known, so the area reads as a
      // share of the machine rather than as a shape fitted to whatever the
      // last minute happened to contain.
      ...(total > 0 ? { max: total } : {}),
      splitLine: { lineStyle: { color: INK.gridline } },
      axisLabel: {
        color: INK.muted,
        fontSize: 10,
        formatter: (value: number) => formatBytes(value),
      },
    },
    series: [
      {
        name: 'Used',
        type: 'line',
        showSymbol: false,
        lineStyle: { width: 2, color: SERIES[0] },
        areaStyle: { color: SERIES[0], opacity: AREA_OPACITY },
        data: used.map((point) => [point.at, point.value]),
      },
    ],
  };
}

/**
 * Two throughput series on one shared axis.
 *
 * @param down Bytes per second received.
 * @param up Bytes per second transmitted.
 */
export function throughputOption(
  down: readonly TimePoint[],
  up: readonly TimePoint[]
): EChartsOption {
  return {
    ...base(formatRate),
    xAxis: timeAxis(),
    yAxis: {
      type: 'value',
      min: 0,
      splitLine: { lineStyle: { color: INK.gridline } },
      axisLabel: {
        color: INK.muted,
        fontSize: 10,
        formatter: (value: number) => formatBytes(value),
      },
    },
    series: [
      {
        name: 'Down',
        type: 'line',
        showSymbol: false,
        lineStyle: { width: 2, color: SERIES[0] },
        areaStyle: { color: SERIES[0], opacity: AREA_OPACITY },
        data: down.map((point) => [point.at, point.value]),
      },
      {
        name: 'Up',
        type: 'line',
        showSymbol: false,
        lineStyle: { width: 2, color: SERIES[1] },
        areaStyle: { color: SERIES[1], opacity: AREA_OPACITY },
        data: up.map((point) => [point.at, point.value]),
      },
    ],
  };
}

/** How many cores are drawn per row before the heatmap wraps. */
export const CORES_PER_ROW = 16;

/**
 * A heatmap of per-core utilisation.
 *
 * One hue, light to dark, because this is magnitude across a grid rather than a
 * set of distinct series.
 *
 * @param perCore Utilisation per logical core, in core order.
 */
export function perCoreHeatmapOption(perCore: readonly number[]): EChartsOption {
  const rows = Math.max(1, Math.ceil(perCore.length / CORES_PER_ROW));
  const data = perCore.map((value, index) => [
    index % CORES_PER_ROW,
    Math.floor(index / CORES_PER_ROW),
    value,
  ]);

  return {
    animation: false,
    grid: { left: 4, right: 4, top: 4, bottom: 4, containLabel: false },
    tooltip: {
      backgroundColor: INK.surface,
      borderColor: INK.axis,
      textStyle: { color: INK.primary, fontSize: 11 },
      formatter: (params: unknown) => {
        const point = params as { data: [number, number, number] };
        const [column, row, value] = point.data;
        return `Core ${row * CORES_PER_ROW + column} — ${formatPercent(value)}`;
      },
    },
    xAxis: {
      type: 'category',
      data: Array.from({ length: Math.min(CORES_PER_ROW, perCore.length) }, (_, i) => String(i)),
      show: false,
    },
    yAxis: {
      type: 'category',
      data: Array.from({ length: rows }, (_, i) => String(i)),
      show: false,
    },
    visualMap: {
      min: 0,
      max: 100,
      show: false,
      // Dark for idle, bright for busy: the ramp's low end recedes toward the
      // surface it is drawn on, which here is dark.
      inRange: { color: SEQUENTIAL_ON_DARK },
    },
    series: [
      {
        type: 'heatmap',
        data,
        itemStyle: {
          // A 2px surface gap between cells, so adjacent cores stay countable.
          borderColor: INK.surface,
          borderWidth: 2,
          borderRadius: 2,
        },
      },
    ],
  };
}
