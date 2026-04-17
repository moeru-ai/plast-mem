const fs = require('fs');
const RESULT_DIR = 'benchmarks/locomo/results/2026-04-14T20-17-43-139Z';
const samples = ['conv-26','conv-30','conv-41','conv-42','conv-43','conv-44','conv-47','conv-48','conv-49','conv-50'];
let allItems = [];
for (const s of samples) {
  const data = JSON.parse(fs.readFileSync(RESULT_DIR + '/samples/' + s + '.json', 'utf-8'));
  allItems.push(...data.variants.plastmem.results.map(r => Object.assign({}, r, {sample: s})));
}

// Multi-hop PARTIAL: is the gold answer actually IN context?
const mh = allItems.filter(r => r.category === 1 && (r.llm_judge_score || 0) === 0.5);
console.log('=== MULTI-HOP PARTIAL: gold in context? ===');
let goldInCtxCount = 0, goldMissingCount = 0;
mh.forEach(r => {
  const ctx = (r.context_retrieved || '').toLowerCase();
  const gold = String(r.gold_answer).toLowerCase();
  // split gold into items
  const items = gold.split(/,|\band\b/).map(s => s.trim().replace(/['"]/g, '')).filter(s => s.length > 2);
  const missing = items.filter(item => !ctx.includes(item));
  if (missing.length > 0) goldMissingCount++;
  else goldInCtxCount++;
});
console.log('Gold fully in context:', goldInCtxCount, '/ missing from context:', goldMissingCount, '/ total:', mh.length);

// Show cases where gold IS in context but model missed it
console.log('\n--- Gold IN context but model partial ---');
let shown = 0;
for (const r of mh) {
  if (shown >= 5) break;
  const ctx = (r.context_retrieved || '').toLowerCase();
  const gold = String(r.gold_answer).toLowerCase();
  const items = gold.split(/,|\band\b/).map(s => s.trim().replace(/['"]/g, '')).filter(s => s.length > 2);
  const missing = items.filter(item => !ctx.includes(item));
  if (missing.length === 0) {
    const episodeCount = (r.context_retrieved.match(/Spoken At:/g) || []).length;
    const factCount = (r.context_retrieved.match(/^- /mg) || []).length;
    console.log('Q:', r.question);
    console.log('Gold:', String(r.gold_answer).substring(0, 100));
    console.log('Pred:', String(r.prediction).substring(0, 100));
    console.log('episodes_in_ctx:', episodeCount, 'facts_in_ctx:', factCount);
    console.log('');
    shown++;
  }
}

// Show cases where gold is MISSING from context
console.log('--- Gold MISSING from context ---');
shown = 0;
for (const r of mh) {
  if (shown >= 5) break;
  const ctx = (r.context_retrieved || '').toLowerCase();
  const gold = String(r.gold_answer).toLowerCase();
  const items = gold.split(/,|\band\b/).map(s => s.trim().replace(/['"]/g, '')).filter(s => s.length > 2);
  const missing = items.filter(item => !ctx.includes(item));
  if (missing.length > 0) {
    const episodeCount = (r.context_retrieved.match(/Spoken At:/g) || []).length;
    console.log('Q:', r.question);
    console.log('Gold:', String(r.gold_answer).substring(0, 100));
    console.log('Pred:', String(r.prediction).substring(0, 100));
    console.log('Missing items:', missing.join(', '));
    console.log('episodes_in_ctx:', episodeCount);
    console.log('');
    shown++;
  }
}

// Temporal: pre-window events
console.log('=== TEMPORAL LLM=0: pre-window events? ===');
const temporal = allItems.filter(r => r.category === 2 && (r.llm_judge_score || 0) === 0);
temporal.slice(0, 8).forEach(r => {
  const ctx = (r.context_retrieved || '').toLowerCase();
  const gold = String(r.gold_answer).toLowerCase();
  // Check if gold date appears in context
  const years = gold.match(/20\d\d/g) || [];
  const goldInCtx = years.some(y => ctx.includes(y));
  console.log('Q:', r.question.substring(0, 70));
  console.log('Gold:', String(r.gold_answer).substring(0, 60), '| gold_in_ctx:', goldInCtx);
  console.log('Pred:', String(r.prediction).substring(0, 60));
  console.log('');
});
