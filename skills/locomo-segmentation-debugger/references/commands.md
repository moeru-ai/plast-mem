# Commands

This project environment has `node` and `python3`. Do not assume `jq` is installed.

`locomo_segmenter` is not a pure local parser. It calls the configured embedding API, and in Codex's sandbox that request may fail against a host-local provider such as `http://localhost:11434`. If you see embedding request errors, re-run the `cargo run ... locomo_segmenter ...` command with escalated permissions before diagnosing segmentation behavior.

## Targeted Run

Use `conv-47` first for segmentation iteration:

```bash
cargo run -q -p plastmem_event_segmentation --example locomo_segmenter -- --sample-id conv-47 > /tmp/conv-47.segments.json
```

If you need to inspect flattened event construction as well:

```bash
cargo run -q -p plastmem_event_segmentation --example locomo_segmenter -- --sample-id conv-47 --print-events > /tmp/conv-47.segments.json
```

`stdout` becomes `/tmp/conv-47.segments.json`. `stderr` still shows sample metadata and warnings.

## Pretty Print

Use Python for a quick pretty-print:

```bash
python3 -m json.tool /tmp/conv-47.segments.json
```

## Pipe Inspection With Node

Print the segment count:

```bash
cat /tmp/conv-47.segments.json | node -e 'let s="";process.stdin.on("data",d=>s+=d);process.stdin.on("end",()=>{const x=JSON.parse(s);console.log(x.length)})'
```

Print the first segment:

```bash
cat /tmp/conv-47.segments.json | node -e 'let s="";process.stdin.on("data",d=>s+=d);process.stdin.on("end",()=>{const x=JSON.parse(s);console.log(JSON.stringify(x[0],null,2))})'
```

Print only boundary reasons and event counts:

```bash
cat /tmp/conv-47.segments.json | node -e 'let s="";process.stdin.on("data",d=>s+=d);process.stdin.on("end",()=>{const x=JSON.parse(s).map((seg,i)=>({index:i,reasons:seg.reasons,events:seg.events.length,score:seg.score}));console.log(JSON.stringify(x,null,2))})'
```

Print one event window from one segment:

```bash
cat /tmp/conv-47.segments.json | node -e 'let s="";process.stdin.on("data",d=>s+=d);process.stdin.on("end",()=>{const x=JSON.parse(s);console.log(JSON.stringify(x[3].events.slice(0,3),null,2))})'
```

## REPL Inspection Or Mutation

Use a `node` REPL when you want to poke at the JSON interactively:

```bash
node
```

Then:

```js
const fs = require('node:fs')

const segments = JSON.parse(fs.readFileSync('/tmp/conv-47.segments.json', 'utf8'))

segments.length
segments[0].reasons
segments[3].events.slice(0, 2)
```

Write a derived file after trimming or rewriting fields:

```js
const trimmed = segments.map(({ events, reasons, score }) => ({ events: events.slice(0, 2), reasons, score }))
fs.writeFileSync('/tmp/conv-47.trimmed.json', JSON.stringify(trimmed, null, 2))
```

Use this for analysis artifacts only. Do not treat edited JSON as input to the debugger.

## All-Sample Sweep

If the question is about overall quality or regressions, run every sample instead of only `conv-47`:

```bash
mkdir -p /tmp/locomo-segments
node -e "const fs=require('node:fs');const data=JSON.parse(fs.readFileSync('benchmarks/locomo/data/locomo10.json','utf8'));for(const sample of data){console.log(sample.sample_id)}" | while read -r sample_id; do cargo run -q -p plastmem_event_segmentation --example locomo_segmenter -- --sample-id \"$sample_id\" > \"/tmp/locomo-segments/$sample_id.json\"; done
```

After that, inspect counts across all samples:

```bash
node -e "const fs=require('node:fs');const path='\/tmp\/locomo-segments';for(const name of fs.readdirSync(path).filter(x=>x.endsWith('.json')).sort()){const segments=JSON.parse(fs.readFileSync(path+'/'+name,'utf8'));console.log(name.replace(/\\.json$/,''), segments.length)}"
```

## When To Choose What

- Use `conv-47` for rapid iteration.
- Use `--print-events` when the problem may be in sample flattening rather than segmentation.
- Use saved JSON plus `node` pipe commands when you need one-off slices.
- Use the `node` REPL when the question is exploratory and you do not yet know which fields matter.
- Use the all-sample sweep when judging broad quality, regressions, or distribution changes.
