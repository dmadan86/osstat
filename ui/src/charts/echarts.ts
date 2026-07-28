/**
 * The ECharts build osstat actually uses.
 *
 * Importing from `echarts` pulls every chart type and component — around 330 KB
 * gzipped — into a desktop app whose whole installer budget is 20 MB. This
 * module registers only what the dashboard renders, and everything else imports
 * from here rather than from `echarts` directly.
 *
 * Adding a chart type means adding it here. That friction is deliberate: it is
 * the moment to ask whether the new form is worth its weight.
 */

import * as echarts from 'echarts/core';
import { BarChart, HeatmapChart, LineChart } from 'echarts/charts';
import {
  GridComponent,
  MarkLineComponent,
  TooltipComponent,
  VisualMapComponent,
} from 'echarts/components';
import { CanvasRenderer } from 'echarts/renderers';

echarts.use([
  LineChart,
  BarChart,
  HeatmapChart,
  GridComponent,
  TooltipComponent,
  VisualMapComponent,
  MarkLineComponent,
  CanvasRenderer,
]);

export { echarts };
export type { EChartsOption } from 'echarts';
