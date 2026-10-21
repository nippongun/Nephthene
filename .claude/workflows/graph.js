export const meta = {
  name: 'graph',
  description: 'Run a planner-written task graph: a Haiku executor per node in dependency order, a fresh Sonnet verifier per node (one retry on refutation), then a final verifier',
  whenToUse: 'After the planner has written the graph. args: {goal, nodes: [{id, deps, files, task, check}], final_check}',
  phases: [
    { title: 'Execute', detail: 'one executor per node, as soon as its deps pass', model: 'haiku' },
    { title: 'Verify', detail: 'fresh verifier per node tries to refute', model: 'sonnet' },
    { title: 'Final', detail: 'fresh verifier over the whole change', model: 'sonnet' },
  ],
}

const REPORT = {
  type: 'object',
  properties: {
    changed: { type: 'array', items: { type: 'string' } },
    check_output: { type: 'string' },
    blocked: { type: 'string' },
  },
  required: ['changed', 'check_output'],
}
const VERDICT = {
  type: 'object',
  properties: {
    pass: { type: 'boolean' },
    evidence: { type: 'string' },
    problems: { type: 'array', items: { type: 'string' } },
  },
  required: ['pass', 'evidence', 'problems'],
}

const g = args
if (!g || !Array.isArray(g.nodes) || !g.nodes.length) throw new Error('args: {goal, nodes: [{id, deps, files, task, check}], final_check}')
const byId = Object.fromEntries(g.nodes.map(n => [n.id, n]))
for (const n of g.nodes) for (const d of n.deps || []) if (!byId[d]) throw new Error(`${n.id}: unknown dep ${d}`)

// Topological order (throws on a cycle), so every dependency's promise exists before its dependants start.
const order = []
const state = {}
const visit = (id, path) => {
  if (state[id] === 'done') return
  if (state[id] === 'open') throw new Error(`cycle: ${[...path, id].join(' -> ')}`)
  state[id] = 'open'
  for (const d of byId[id].deps || []) visit(d, [...path, id])
  state[id] = 'done'
  order.push(byId[id])
}
for (const n of g.nodes) visit(n.id, [])

const executorPrompt = (n, refuted) => [
  `Goal: ${g.goal}`,
  `Node ${n.id}: ${n.task}`,
  `Files you may touch: ${(n.files || []).join(', ') || '(none: read-only node)'}`,
  `Done when: ${n.check}`,
  refuted ? `Your previous attempt was refuted by the verifier. Fix exactly this:\n${refuted}` : '',
].filter(Boolean).join('\n\n')

const verifyPrompt = (n, rep) => [
  `Claim: node ${n.id} of "${g.goal}" is done.`,
  `Task: ${n.task}`,
  `Allowed files: ${(n.files || []).join(', ') || '(none)'}`,
  `Check: ${n.check}`,
  `Executor says it changed: ${rep.changed.join(', ') || '(nothing)'}; check output:\n${rep.check_output}`,
  'Re-run the check and read the diff. Refute if anything is off.',
].join('\n\n')

// Role agents (.claude/agents) load at session start; a session that predates them gets the same model told to read its role file.
const MODEL = { executor: 'haiku', verifier: 'sonnet' }
const role = async (kind, prompt, opts) => {
  try { return await agent(prompt, { ...opts, agentType: kind }) } catch (e) {
    if (!String(e).includes('not found')) throw e
    return agent(`You are the ${kind}: read .claude/agents/${kind}.md and follow it exactly.\n\n${prompt}`, { ...opts, model: MODEL[kind] })
  }
}

const promises = {}
const run = async n => {
  const deps = await Promise.all((n.deps || []).map(d => promises[d]))
  if (deps.some(r => !r || !r.pass)) { log(`${n.id}: skipped (a dependency failed)`); return { id: n.id, pass: false, skipped: true } }
  let refuted = ''
  for (let attempt = 1; attempt <= 2; attempt++) {
    const rep = await role('executor', executorPrompt(n, refuted), { label: `exec ${n.id} #${attempt}`, phase: 'Execute', schema: REPORT })
    if (!rep) return { id: n.id, pass: false, problems: ['executor died'] }
    if (rep.blocked) { log(`${n.id}: blocked: ${rep.blocked}`); return { id: n.id, pass: false, blocked: rep.blocked } }
    const v = await role('verifier', verifyPrompt(n, rep), { label: `verify ${n.id} #${attempt}`, phase: 'Verify', schema: VERDICT })
    if (v && v.pass) return { id: n.id, pass: true, attempt, changed: rep.changed, evidence: v.evidence }
    refuted = v ? v.problems.join('\n') : 'verifier died'
    log(`${n.id}: attempt ${attempt} refuted: ${refuted.slice(0, 160)}`)
  }
  return { id: n.id, pass: false, problems: refuted.split('\n') }
}
for (const n of order) promises[n.id] = run(n)
const results = await Promise.all(order.map(n => promises[n.id]))

const failed = results.filter(r => !r.pass)
let final = null
if (!failed.length && g.final_check) {
  final = await role('verifier', [
    `Claim: "${g.goal}" is done; every node passed its own check.`,
    `Final check: ${g.final_check}`,
    `Nodes: ${results.map(r => `${r.id} (${(r.changed || []).join(', ')})`).join('; ')}`,
    'Run the final check, read the whole diff against HEAD, and refute if the change as a whole is wrong, incomplete or breaks a rule.',
  ].join('\n\n'), { label: 'final verify', phase: 'Final', schema: VERDICT })
}
return { pass: !failed.length && (!g.final_check || !!(final && final.pass)), results, final }
