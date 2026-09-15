import { spawnSync } from 'node:child_process';
import { existsSync, renameSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const skill = process.env.ARCHIFY_HOME || path.join(process.env.CODEX_HOME || path.join(homedir(), '.codex'), 'skills', 'archify');
const cli = path.join(skill, 'bin', 'archify.mjs');
const source = path.join(here, 'backend.architecture.json');
const output = path.join(here, 'backend.html');
const mode = process.argv[2] || 'render';
if (!['render', 'validate', 'preview', 'visual-check'].includes(mode)) {
  console.error('Usage: node update.mjs [render|validate|preview|visual-check]');
  process.exit(2);
}
if (!existsSync(cli)) {
  console.error('Archify 未找到。请安装技能，或设置 ARCHIFY_HOME 指向技能目录。');
  process.exit(2);
}

function run(args, receipt) {
  const result = spawnSync(process.execPath, [cli, ...args], {
    cwd: here,
    encoding: 'utf8',
    stdio: receipt ? 'pipe' : 'inherit',
    maxBuffer: 16 * 1024 * 1024,
  });
  if (result.error) console.error(result.error.message);
  if (receipt) {
    if (result.stdout) process.stdout.write(result.stdout);
    if (result.stderr) process.stderr.write(result.stderr);
  }
  if (result.status !== 0) process.exit(result.status || 1);
  if (receipt) {
    const parsed = JSON.parse(result.stdout);
    if (parsed.ok !== true) throw new Error('Archify 未返回成功回执');
    const target = path.join(here, receipt);
    const temporary = `${target}.${process.pid}.tmp`;
    writeFileSync(temporary, result.stdout);
    renameSync(temporary, target);
  }
}

if (mode === 'render' || mode === 'validate') {
  run(['validate', 'architecture', source, '--quality', 'showcase', '--json'], 'validation.receipt.json');
}
if (mode === 'render') {
  run(['deliver', 'architecture', source, output, '--quality', 'showcase', '--json'], 'delivery.receipt.json');
  console.log('已更新 backend.html；如需重新验收浏览器显示，执行 node update.mjs visual-check。');
} else if (mode === 'preview') {
  run(['preview', 'architecture', source, output, '--quality', 'showcase']);
} else if (mode === 'visual-check') {
  run(['visual-check', output, '--json']);
}
