/**
 * The bridge between React and ECharts.
 *
 * Small on purpose, and hand-written rather than pulled from a wrapper library:
 * all this needs to do is own an instance, push options, follow resizes and
 * dispose cleanly, and a dependency for that would cost more than it saves.
 */

import { useEffect, useRef } from 'react';

import { echarts, type EChartsOption } from '../charts/echarts';

/** Everything a chart needs to render. */
export interface ChartProps {
  /** The option object, built by a pure function in `charts/options.ts`. */
  option: EChartsOption;
  /** CSS height. Charts need an explicit height; a canvas has no intrinsic one. */
  height: number;
  /** Describes the chart for anyone who cannot see it. */
  label: string;
}

/**
 * Renders an ECharts chart.
 *
 * @param props The option, height and accessible label.
 */
export function Chart({ option, height, label }: ChartProps): React.JSX.Element {
  const container = useRef<HTMLDivElement | null>(null);
  const instance = useRef<echarts.ECharts | null>(null);

  // Held so a chart created after its first options arrive still gets them.
  const pending = useRef(option);

  useEffect(() => {
    const element = container.current;
    if (element === null) return undefined;

    // ResizeObserver rather than a window listener: the chart also has to
    // follow the sidebar collapsing and a section expanding, neither of which
    // resizes the window.
    const observer = new ResizeObserver(() => {
      if (element.clientWidth === 0 || element.clientHeight === 0) return;

      // Creating the chart before the element has been laid out produces a
      // zero-sized canvas and an ECharts warning, which happens routinely for a
      // section that mounts collapsed or behind an unselected sub-tab. Waiting
      // for a real size costs nothing and keeps the console meaningful.
      if (instance.current === null) {
        instance.current = echarts.init(element, undefined, { renderer: 'canvas' });
        instance.current.setOption(pending.current);
      } else {
        instance.current.resize();
      }
    });
    observer.observe(element);

    return () => {
      observer.disconnect();
      instance.current?.dispose();
      instance.current = null;
    };
  }, []);

  useEffect(() => {
    // Recorded for a chart that has not been created yet, because its element
    // had no size when the observer first fired.
    pending.current = option;
    // `notMerge: false` lets a tick replace series data without rebuilding the
    // axes, which is what keeps a 2 s refresh from flickering.
    instance.current?.setOption(option, { notMerge: false, lazyUpdate: true });
  }, [option]);

  return <div ref={container} role="img" aria-label={label} style={{ height, width: '100%' }} />;
}
