import fs from 'node:fs';
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

process.chdir(fileURLToPath(new URL('../', import.meta.url)));
const hash = (b) => createHash('sha256').update(b).digest('hex');
const graph = JSON.parse(fs.readFileSync('compass-out/graph.json'));
const scope = JSON.parse(fs.readFileSync('.compass/scope.json'));
const extracted = graph.graph.files.filter((f) => f.extractionStatus === 'extracted');
const actual = extracted.map((f) => f.path).sort();
const missing = scope.expectedFiles.filter((p) => !actual.includes(p));
const unexpected = actual.filter((p) => !scope.expectedFiles.includes(p));
const stale = extracted.filter((f) => f.contentDigest !== `sha256:${hash(fs.readFileSync(f.path))}`);
if (missing.length || unexpected.length || stale.length) throw Error('Graph scope/freshness mismatch');
const quality = JSON.parse(execFileSync('/usr/local/bin/compass', ['diagnose', 'quality', '--graph', 'compass-out/graph.json', '--json'], { encoding: 'utf8' }));
const config = JSON.parse(fs.readFileSync('.mcp.json')).mcpServers['compass-surreal-memory-server'];
const child = spawn(config.command, config.args, { stdio: ['pipe', 'pipe', 'pipe'] });
let nextId = 0, buffer = '', stderr = '';
const pending = new Map();
const timeout = setTimeout(() => {
  for (const p of pending.values()) p.reject(Error('MCP response timeout after 30 seconds'));
  child.kill();
}, 30000);
child.stderr.on('data', (b) => { stderr += b; });
child.stdout.on('data', (b) => {
  buffer += b;
  let i;
  while ((i = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, i); buffer = buffer.slice(i + 1);
    if (!line.trim()) continue;
    const response = JSON.parse(line), p = pending.get(response.id);
    if (p) { pending.delete(response.id); response.error ? p.reject(Error(JSON.stringify(response.error))) : p.resolve(response.result); }
  }
});
function call(method, params = {}) {
  return new Promise((resolve, reject) => {
    const id = ++nextId; pending.set(id, { resolve, reject });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params: { ...params, _meta: {
      'io.modelcontextprotocol/protocolVersion': '2026-07-28',
      'io.modelcontextprotocol/clientInfo': { name: 'memory-graph-verification', version: '1' },
      'io.modelcontextprotocol/clientCapabilities': {},
    } } }) + '\n');
  });
}
const receipts = [];
try {
  receipts.push({ method: 'server/discover', result: await call('server/discover') });
  const tools = await call('tools/list');
  receipts.push({ method: 'tools/list', result: tools });
  for (const [name, args] of [
    ['graph_stats', {}],
    ['search_symbols', { query: 'build_router' }],
    ['get_callees', { symbol: 'surreal_memory_server::api::build_router' }],
  ]) {
    const result = await call('tools/call', { name, arguments: args });
    if (result.isError) throw Error(`MCP ${name} returned an error`);
    receipts.push({ method: name, args, result });
  }
  const callee = receipts.at(-1);
  if (!JSON.stringify(callee.result).includes('OperationService::start')) throw Error('Expected representative call missing from MCP response');
  const nodes = new Map(graph.nodes.map((n) => [n.id, n]));
  const pairs = [
    ['surreal_memory_server::main', 'surreal_memory_server::run_api_server'],
    ['surreal_memory_server::api::build_router', 'surreal_memory_server::operations::OperationService::start'],
    ['surreal_memory_server::operations::payload_hash', 'surreal_memory_server::operations::digest_hex'],
    ['surreal_memory_server::executor::run_embedding_executor', 'surreal_memory_server::executor::execute_request'],
    ['surreal_memory_server::mcp::MemoryMcpServer::add_mindmap_node', 'surreal_memory_server::mcp::progress::run_with_progress'],
  ];
  const representativeCalls = pairs.map(([from, to]) => {
    const edge = graph.links.find((e) => e.kind === 'calls' && nodes.get(e.source)?.qualifiedName === from && nodes.get(e.target)?.qualifiedName === to);
    if (!edge) throw Error(`Expected call missing: ${from} -> ${to}`);
    return { from, to, site: edge.relationshipSite, edgeId: edge.id };
  });
  fs.mkdirSync('compass-out/verification', { recursive: true });
  fs.writeFileSync('compass-out/verification/mcp.json', JSON.stringify({ config, receipts, stderr }, null, 2) + '\n');
  const report = {
    checkedAt: new Date().toISOString(), sourceCommit: graph.graph.build.sourceCommit,
    graphSha256: hash(fs.readFileSync('compass-out/graph.json')), build: graph.graph.build,
    files: actual.length, missing, unexpected, stale: [], extractionFailures: graph.graph.files.filter((f) => !['extracted', 'excluded'].includes(f.extractionStatus)),
    excludedMetadata: graph.graph.files.filter((f) => f.extractionStatus === 'excluded').map(({ path, extractionStatus }) => ({ path, extractionStatus })),
    quality, representativeCalls, mcp: { command: config, discoveredTools: tools.tools.map((t) => t.name), successfulQueries: receipts.slice(2).map((r) => r.method), protocol: '2026-07-28' },
    limits: [...scope.limits, '134 test relations confirm inline tests are present; counts are not production-only symbol counts.', 'Graph exact confidence is extractor evidence, not proof of runtime dispatch or complete reachability.'],
  };
  fs.writeFileSync('.compass/verification.json', JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify({ files: report.files, nodes: quality.node_count, edges: quality.edge_count, calls: quality.edge_kinds.calls, tools: tools.tools.length, queries: report.mcp.successfulQueries }));
} finally { clearTimeout(timeout); child.kill(); }
