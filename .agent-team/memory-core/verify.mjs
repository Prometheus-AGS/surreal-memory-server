import fs from 'node:fs';
import { spawnSync, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

process.chdir(fileURLToPath(new URL('../../', import.meta.url)));
const require = createRequire(import.meta.url);
// Reuse the existing workspace parser read-only; this verifier installs nothing.
const toml = require(require.resolve('smol-toml', { paths: ['../the-boss'] }));
const base = '.agent-team/memory-core';
const team = JSON.parse(fs.readFileSync(`${base}/team.json`));
const hash = (b) => createHash('sha256').update(b).digest('hex');
const checks = [];
for (const role of team.roles) {
  for (const target of ['codex', 'claude', 'kimi', 'opencode', 'minimax']) {
    const rel = target === 'codex' ? `.codex/agents/${role.id}.toml`
      : target === 'claude' ? `.claude/agents/${role.id}.md`
      : target === 'kimi' ? `.kimi-code/agents/${role.id}.md`
      : target === 'opencode' ? `.opencode/agents/${role.id}.md`
      : `.minimax/agents/${role.id}/agent.md`;
    const text = fs.readFileSync(rel, 'utf8');
    const parsed = target === 'codex' ? toml.parse(text) : JSON.parse(text.split('\n---\n')[0].slice(4));
    const staged = `${base}/exports/${target}/${target === 'minimax' ? rel.replace('.minimax/', '') : rel}`;
    if (fs.readFileSync(staged, 'utf8') !== text) throw Error(`Export differs: ${rel}`);
    const name = target === 'opencode' ? role.id : parsed.name;
    if (name !== role.id) throw Error(`Name mismatch: ${rel}`);
    const prompt = target === 'codex' ? parsed.developer_instructions : text.split('\n---\n').slice(1).join('\n---\n');
    if (!prompt.includes(role.prompt)) throw Error(`Prompt mismatch: ${rel}`);
    checks.push({ target, path: rel, name, syntax: 'passed', promptMatches: true, sha256: hash(text),
      nativeEnvelope: Object.fromEntries(Object.entries(parsed).filter(([k]) => k !== 'developer_instructions')) });
  }
  for (const skill of role.skills) if (!fs.existsSync(`.agents/skills/${skill}/SKILL.md`)) throw Error(`Missing skill: ${skill}`);
}
const baseline = JSON.parse(fs.readFileSync(`${base}/research/baseline.json`));
const preservation = [];
for (const [path, before] of Object.entries(baseline.files)) {
  let current = fs.readFileSync(path);
  const appended = ['AGENTS.md', 'CLAUDE.md'].includes(path);
  if (appended) current = current.subarray(0, before.bytes);
  if (hash(current) !== before.sha256) throw Error(`Preexisting bytes changed: ${path}`);
  preservation.push({ path, preserved: true, mode: appended ? 'original bytes retained; routing appended' : 'byte identical' });
}
const mcp = JSON.parse(fs.readFileSync('.mcp.json')).mcpServers['compass-surreal-memory-server'];
const codex = toml.parse(fs.readFileSync('.codex/config.toml', 'utf8')).mcp_servers['compass-surreal-memory-server'];
if (JSON.stringify(mcp) !== JSON.stringify(codex)) throw Error('MCP definitions differ');
for (const skill of JSON.parse(fs.readFileSync(`${base}/skills-lock.json`)).copied) {
  for (const file of skill.files) if (hash(fs.readFileSync(file.path)) !== file.sha256) throw Error(`Skill changed: ${file.path}`);
}
const validated = JSON.parse(execFileSync(process.execPath, ['.agents/skills/agent-team-creator/scripts/cli.mjs', 'validate', '--input', `${base}/team-request.json`], { encoding: 'utf8' }));
if (!validated.valid) throw Error('Invalid manifest');
const report = {
  schemaVersion: 1, checkedAt: new Date().toISOString(), roleCount: team.roles.length,
  nativeCount: checks.length, manifestValidation: 'creator CLI passed', nativeSyntax: checks,
  exportBytesMatch: true, skillBindings: 'all present', skillHashes: 'all matched',
  mcpDefinitions: 'matching exact project graph command', preservation,
  nativeDiscovery: {},
  limits: ['No live role/model execution.', 'Source contracts and serialization do not prove all installed native discovery.',
    'No application code or dependency manifests changed; no Cargo execution.',
    'This optional syntax verifier uses the existing sibling Boss smol-toml parser read-only; agent definitions and launcher do not depend on it.'],
};
fs.mkdirSync('compass-out/verification', { recursive: true });
const fd = fs.openSync('compass-out/verification/opencode-native.json', 'w');
const oc = spawnSync('opencode', ['debug', 'agent', 'memory-lead', '--pure'], { stdio: ['ignore', fd, 'pipe'], timeout: 20000 });
fs.closeSync(fd);
if (oc.status === 0) {
  const native = JSON.parse(fs.readFileSync('compass-out/verification/opencode-native.json'));
  report.nativeDiscovery.opencode = { exitCode: 0, name: native.name, mode: native.mode, promptMatches: native.prompt?.includes(team.roles[0].prompt) };
} else report.nativeDiscovery.opencode = { exitCode: oc.status, signal: oc.signal, error: oc.error?.code };
report.nativeDiscovery.claude = { status: 'source-and-syntax-only', limitation: 'This installed claude agents --json lists active sessions, not custom role definitions; it is not a discovery check.' };
report.nativeDiscovery.otherHarnesses = 'Source contracts and serialization checked; custom-role discovery/execution not tested. MiniMax launcher --version returned 0.5.4.';
report.installedVersions = { codex: '0.154.0', claude: '2.1.282', kimi: '0.42.0', opencode: '1.18.25-fork', minimax: '0.5.4', provenance: 'CLI version calls on 2026-09-24; mcode via project launcher' };
fs.writeFileSync(`${base}/verification.json`, JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify({ roleCount: report.roleCount, nativeCount: checks.length, preserved: preservation.length, nativeDiscovery: report.nativeDiscovery }));
