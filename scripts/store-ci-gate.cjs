// Used by actions/github-script. Ordinary push CI omits native crash suites.
module.exports = async ({github, context, core}) => {
  const required = ['fmt', 'deny', 'Windows packaging checks'];
  for (const os of ['ubuntu-latest', 'windows-latest', 'macos-latest']) {
    required.push(`clippy + test (${os})`, `crash suite (${os})`);
  }
  const runs = await github.paginate(github.rest.actions.listWorkflowRuns, {
    ...context.repo, workflow_id: 'ci.yml', head_sha: context.sha,
    event: 'workflow_dispatch', per_page: 100,
  });
  // A failed later manual run invalidates an older success on the same SHA.
  const run = runs.sort((a, b) => b.id - a.id)[0];
  if (!run || run.head_sha !== context.sha || run.conclusion !== 'success') {
    throw new Error(`Run manual CI successfully on ${context.sha} before Store submission.`);
  }
  const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRun, {
    ...context.repo, run_id: run.id, filter: 'latest', per_page: 100,
  });
  for (const name of required) {
    if (!jobs.some(job => job.name === name && job.conclusion === 'success')) {
      throw new Error(`Manual CI is missing a successful required job: ${name}`);
    }
  }
  core.info(`Required CI passed: ${run.html_url}`);
};
