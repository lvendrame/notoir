const {test} = require('node:test');
const assert = require('node:assert/strict');
const gate = require('../store-ci-gate.cjs');
const sha = 'candidate';
const success = {id: 1, head_sha: sha, conclusion: 'success', html_url: 'https://example.test/ci'};
const jobs = ['fmt', 'deny', 'Windows packaging checks',
  ...['ubuntu-latest', 'windows-latest', 'macos-latest'].flatMap(os =>
    [`clippy + test (${os})`, `crash suite (${os})`])]
  .map(name => ({name, conclusion: 'success'}));
function setup(runs, jobResults = jobs) {
  return {
    github: {
      rest: {actions: {listWorkflowRuns: 'runs', listJobsForWorkflowRun: 'jobs'}},
      paginate: async (endpoint, args) => {
        if (endpoint === 'runs') {
          assert.equal(args.head_sha, sha);
          assert.equal(args.event, 'workflow_dispatch');
          assert.equal(args.workflow_id, 'ci.yml');
          return runs;
        }
        assert.equal(args.run_id, 1);
        return jobResults;
      },
    },
    context: {sha, repo: {owner: 'example', repo: 'app'}},
    core: {info() {}},
  };
}
test('requires a manual run on the candidate, including every platform', async () => {
  await assert.rejects(gate(setup([])), /Run manual CI/);
  await assert.rejects(gate(setup([{...success, head_sha: 'old'}])), /Run manual CI/);
  for (const job of jobs) {
    await assert.rejects(gate(setup([success], jobs.filter(j => j !== job))), /missing a successful/);
    await assert.rejects(gate(setup([success], jobs.map(j => j === job ? {...j, conclusion: 'skipped'} : j))), /missing a successful/);
  }
  await gate(setup([success]));
});
test('a newer failed or running manual run invalidates an older success', async () => {
  for (const conclusion of ['failure', null, 'cancelled']) {
    await assert.rejects(gate(setup([success, {...success, id: 2, conclusion}])), /Run manual CI/);
  }
});
