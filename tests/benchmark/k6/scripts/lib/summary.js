// benchmark-action compatible output formatter
// Outputs JSON array for github-action-benchmark
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

    const output = [
      {
        name: `${scriptName}_iterations_per_second`,
        unit: 'Number',
        value: Math.round(iterationsPerSecond * 100) / 100,
      },
      {
        name: `${scriptName}_p95_ms`,
        unit: 'ms',
        value: Math.round(p95Duration * 100) / 100,
      },
    ];

    return {
      'stdout': textSummary(data),
      [`results/k6/${scriptName}-output.json`]: JSON.stringify(output, null, 2),
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
    lines.push(`duration p95: ${dur['p(95)'].toFixed(2)}ms`);
    lines.push(`duration avg: ${dur.avg.toFixed(2)}ms`);
  }

  lines.push('');
  return lines.join('\n');
}
