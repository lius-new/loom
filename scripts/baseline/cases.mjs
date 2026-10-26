// Explicit opt-in cases: keep exact Rust paths and expected measurements together.
const keys = (scenarios, metrics) => scenarios.flatMap(scenario => metrics.map(metric => `${scenario}/${metric}`));
export const BENCHMARKS = [
  {id: 'editor', group: 'editor', filter: 'app::editor_baseline_tests::perf_baseline', aliases: ['legacy-editor'],
    metrics: keys(['ascii-1m', 'ascii-10m'], ['typing_per_char', 'paste_1m', 'undo_paste', 'select_all_indent', 'history_bytes_rounded'])},
  {id: 'ui-interaction', group: 'ui', filter: 'app::editor::tests::large_file_interaction_baseline',
    metrics: keys(['8 MiB single line', 'Unicode single line', 'many short lines'], ['ui_document_prepare', 'ui_100_viewports_and_clicks', 'ui_20_edits_and_renders'])},
  {id: 'tab-pointer', group: 'ui', filter: 'app::tabs::tests::pointer_updates_with_many_tabs',
    metrics: keys(['100-tabs', '500-tabs'], ['pointer_update'])},
  {id: 'file', group: 'file', filter: 'app::phase0_baseline::phase0_perf_file_preparation',
    metrics: keys(['ascii-1m', 'ascii-10m', 'ascii-64m'], ['file_read_cached', 'document_prepare'])},
  {id: 'search', group: 'search', filter: 'app::phase0_baseline::phase0_perf_search_scan_and_cancel',
    metrics: ['100-files-6.25m/search_scan_cached'], optionalMetrics: ['100-files-6.25m/search_cancel_latency']},
  {id: 'terminal', group: 'terminal', filter: 'app::phase0_baseline::phase0_perf_terminal_parser',
    metrics: ['ansi-unicode-8m/terminal_parse', '24x80-screen/terminal_screen_snapshot_100']},
  {id: 'terminal-production', group: 'terminal', filter: 'app::terminal_performance::terminal_production_benchmark',
    metrics: keys([1, 8, 32].map(n => `terminal-${n}-sessions`), ['production_screen_snapshot_100', 'model_snapshot_500', 'frame_projection_500', 'terminal_ui_frame_100'])},
];

export const NATIVE_SMOKES = [
  ['native-terminal-smoke', 'supported_shells_round_trip_through_real_conpty_sessions'],
  ['native-launcher-smoke', 'launcher_backed_shell_is_idle_then_detects_and_clears_programs'],
  ['native-bash-replacement-smoke', 'exited_shell_launchers_do_not_hide_unknown_applications'],
  ['native-program-name-smoke', 'running_application_is_detected_and_cleared_in_each_real_shell'],
].map(([id, name]) => ({id, filter: `app::terminal_session_tests::tests::${name}`}));

export function selectBenchmarks(selection = 'all') {
  if (selection === 'none') return [];
  if (selection === 'all') return BENCHMARKS;
  const groups = selection.split(',');
  if (groups.some(group => !BENCHMARKS.some(item => item.group === group))) throw Error(`Unknown performance group: ${selection}`);
  return BENCHMARKS.filter(item => groups.includes(item.group));
}

export function benchmarkForLog(name) {
  const match = /^perf-(.+)-(\d+)\.log$/.exec(name);
  if (!match) return null;
  const benchmark = BENCHMARKS.find(item => item.id === match[1] || item.aliases?.includes(match[1]));
  return benchmark ? {benchmark, sample: Number(match[2])} : null;
}
