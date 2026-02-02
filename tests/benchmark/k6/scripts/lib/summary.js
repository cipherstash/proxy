// benchmark-action compatible output formatter
// Outputs separate JSON files for throughput (bigger is better) and latency (smaller is better)
//
// Usage in scripts:
//   import { createSummaryHandler } from './lib/summary.js';
//   export const handleSummary = createSummaryHandler('script-name');

export function createSummaryHandler(scriptName) {
  return function(data) {
    const iterationsPerSecond = data.metrics.iterations
      ? data.metrics.iterations.values.rate
      : 0;

    const p95Duration = data.metrics.iteration_duration
      ? data.metrics.iteration_duration.values['p(95)']
      : 0;

    const p99Duration = data.metrics.iteration_duration
      ? data.metrics.iteration_duration.values['p(99)']
      : 0;

    // Throughput metrics (customBiggerIsBetter)
    const throughputOutput = [
      {
        name: `${scriptName}_rate`,
        unit: 'iter/s',
        value: Math.round(iterationsPerSecond * 100) / 100,
      },
    ];

    // Latency metrics (customSmallerIsBetter)
    const latencyOutput = [
      {
        name: `${scriptName}_p95`,
        unit: 'ms',
        value: Math.round(p95Duration * 100) / 100,
      },
      {
        name: `${scriptName}_p99`,
        unit: 'ms',
        value: Math.round(p99Duration * 100) / 100,
      },
    ];

    return {
      'stdout': textSummary(data),
      [`results/k6/${scriptName}-throughput.json`]: JSON.stringify(throughputOutput, null, 2),
      [`results/k6/${scriptName}-latency.json`]: JSON.stringify(latencyOutput, null, 2),
    };
  };
}

// Minimal text summary (k6 doesn't export textSummary by default in extensions)
function textSummary(data) {
  const lines = [];
  lines.push('');
  lines.push('=== Summary ===');

  if (data.metrics.iterations) {
    lines.push(`iterations: ${data.metrics.iterations.values.count}`);
    lines.push(`rate: ${data.metrics.iterations.values.rate.toFixed(2)}/s`);
  }

  if (data.metrics.iteration_duration) {
    const dur = data.metrics.iteration_duration.values;
    lines.push(`duration min: ${dur.min.toFixed(2)}ms`);
    lines.push(`duration avg: ${dur.avg.toFixed(2)}ms`);
    lines.push(`duration p95: ${dur['p(95)'].toFixed(2)}ms`);
    lines.push(`duration p99: ${dur['p(99)'].toFixed(2)}ms`);
    lines.push(`duration max: ${dur.max.toFixed(2)}ms`);
  }

  lines.push('');
  return lines.join('\n');
}
