/**
 * Conventional Commits, enforced on every commit message and on PR titles
 * (squash-merge means the PR title becomes the commit on main). See ADR-010.
 */
export default {
  extends: ['@commitlint/config-conventional'],
  rules: {
    'type-enum': [
      2,
      'always',
      [
        'feat',
        'fix',
        'perf',
        'refactor',
        'docs',
        'test',
        'build',
        'ci',
        'chore',
        'revert',
        // osstat-specific: adding or changing a cleaning rule manifest, which
        // is reviewed under different rules than code (see CODEOWNERS).
        'rules',
      ],
    ],
    'scope-case': [2, 'always', 'kebab-case'],
    'subject-case': [2, 'never', ['sentence-case', 'start-case', 'pascal-case', 'upper-case']],
    'body-max-line-length': [0],
  },
};
