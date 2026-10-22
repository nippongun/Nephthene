export const meta = {
  name: 'graph',
  description: 'Run a planner-written task graph: a Haiku executor per node in dependency order, then one fresh Sonnet verifier over the whole change',
  whenToUse: 'After the planner has written the graph. args: {goal, nodes: [{id, deps, files, task, check}], final_check}',
  phases: [
    { title: 'Execute', detail: 'one executor per node, as soon as its deps pass (each runs its own check)', model: 'haiku' },
    { title: 'Final', detail: 'one fresh verifier over the whole change', model: 'sonnet' },
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

const executorPrompt = n => [
  `Goal: ${g.goal}`,
  `Node ${n.id}: ${n.task}`,
  `Files you may touch: ${(n.files || []).join(', ') || '(none: read-only node)'}`,
  `Done when: ${n.check}`,
].filter(Boolean).join('\n\n')

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
  const rep = await role('executor', executorPrompt(n), { label: `exec ${n.id}`, phase: 'Execute', schema: REPORT })
  if (!rep) return { id: n.id, pass: false, problems: ['executor died'] }
  if (rep.blocked) { log(`${n.id}: blocked: ${rep.blocked}`); return { id: n.id, pass: false, blocked: rep.blocked } }
  return { id: n.id, pass: true, changed: rep.changed, check_output: rep.check_output }
}
for (const n of order) promises[n.id] = run(n)
const results = await Promise.all(order.map(n => promises[n.id]))

const failed = results.filter(r => !r.pass)
let final = null
if (!failed.length) {
  final = await role('verifier', [
    `Claim: "${g.goal}" is done; every node's executor reports its own check passed (unverified).`,
    `Final check: ${g.final_check || '(none beyond the node checks)'}`,
    `Nodes:\n${results.map(r => `${r.id} (${(r.changed || []).join(', ')}): ${(r.check_output || '').slice(0, 400)}`).join('\n')}`,
    'Re-run each node check and the final check, read the whole diff against HEAD, and refute if any node or the change as a whole is wrong, incomplete or breaks a rule.',
  ].join('\n\n'), { label: 'final verify', phase: 'Final', schema: VERDICT })
}
return { pass: !failed.length && !!(final && final.pass), results, final }
