import assert from 'node:assert/strict';

// Validate the parsed workflow, not comments or unrelated command substrings.
export function verifySecurityGate(workflow) {
  const gate = workflow.jobs?.security;
  const publish = workflow.jobs?.publish;
  assert(gate, 'Manual releases require a security job');
  assert(Array.isArray(publish?.needs) && publish.needs.includes('security'),
    'Publishing must depend on security');
  assert.equal(publish.if, undefined, 'Do not override the default successful-needs gate');
  assert.equal(gate['continue-on-error'], undefined, 'Security must fail closed');
  assert.equal(gate.needs, 'prepare');
  assert.equal(gate.if, "needs.prepare.outputs.release == 'true'");
  const manifests = gate.steps.find(step => step.uses?.startsWith('actions/download-artifact@'));
  assert.equal(manifests?.with?.name, 'release-manifests', 'Audit the exact release lockfiles');
  assert.equal(manifests.with.path, '.');
  for (const [id, command] of [
    ['audit-root', 'cargo audit --deny warnings --file Cargo.lock'],
    ['audit-fuzz', 'cargo audit --deny warnings --file fuzz/Cargo.lock'],
  ]) {
    const matches = gate.steps.filter(step => step.id === id);
    assert.equal(matches.length, 1, `${id} must exist exactly once`);
    const step = matches[0];
    assert.equal(step.run, command, 'No suppression or success fallback is allowed');
    assert.equal(step['continue-on-error'], undefined);
    assert.equal(step.if, undefined, 'Audit steps cannot be skipped conditionally');
  }
}

export function testSecurityGate(workflow) {
  verifySecurityGate(workflow);
  for (const mutate of [
    value => { delete value.jobs.security; },
    value => { value.jobs.publish.needs = value.jobs.publish.needs.filter(job => job !== 'security'); },
    value => { value.jobs.publish.if = 'always()'; },
    value => { value.jobs.security['continue-on-error'] = true; },
    value => { value.jobs.security.steps.find(step => step.id === 'audit-root').run += ' || true'; },
    value => { value.jobs.security.steps.find(step => step.id === 'audit-fuzz').if = 'false'; },
    value => { value.jobs.security.steps.find(step => step.uses?.startsWith('actions/download-artifact@')).with.name = 'unrelated'; },
  ]) {
    const candidate = structuredClone(workflow);
    mutate(candidate);
    assert.throws(() => verifySecurityGate(candidate));
  }
}
