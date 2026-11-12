import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const root = process.cwd(), base = process.argv[2];
assert(base && /^\/[\w./-]+$/.test(base) && /^\/[\w./-]+$/.test(root));
const fixture = `${root}/benchmarks/downstream-consumer`, output = `${base}/graphs`;
fs.mkdirSync(output);
const hash = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const inputs = Object.fromEntries(['Cargo.toml', 'Cargo.lock', 'crates/usd_bevy/Cargo.toml',
  'benchmarks/downstream-consumer/Cargo.toml', 'benchmarks/downstream-consumer/Cargo.lock',
  'vendor/bevy_asset/Cargo.toml', 'vendor/bevy_pbr/Cargo.toml'].map(file => [file, hash(file)]));
const one = (data, name) => {
  const values = data.packages.filter(p => p.name === name);
  assert.equal(values.length, 1, `${name} must resolve once`); return values[0];
};
function verify(data, workspace) {
  assert.equal(data.workspace_root, workspace);
  const consumer = one(data, 'usd-downstream-consumer');
  assert.deepEqual(data.workspace_members, [consumer.id]); assert.equal(data.resolve.root, consumer.id);
  assert(data.packages.every(p => !['usdview', 'mara', 'mara_bevy'].includes(p.name)), 'viewer leaked into consumer graph');
  assert.equal(one(data, 'usd_bevy').manifest_path, `${root}/crates/usd_bevy/Cargo.toml`);
  for (const name of ['bevy', 'bevy_internal', 'bevy_asset', 'bevy_pbr']) assert.equal(one(data, name).version, '0.20.1');
  for (const name of ['bevy_asset', 'bevy_pbr']) {
    const pkg = one(data, name);
    assert.equal(pkg.source, null, `${name} must use the consumer's required local patch`);
    assert.equal(fs.realpathSync(pkg.manifest_path), `${root}/vendor/${name}/Cargo.toml`);
  }
  for (const name of ['openusd', 'openusd-schemas']) assert(one(data, name).source.endsWith('#900ea31591348e3fbf416d4661d703aa8700771b'));
  const features = name => data.resolve.nodes.find(n => n.id === one(data, name).id).features;
  assert(features('bevy').includes('morph')); assert(features('bevy_pbr').includes('pbr_multi_layer_material_textures'));
  return { workspace, packages: data.packages.length, bevyFeatures: features('bevy'), libraryFeatures: features('usd_bevy') };
}
function metadata(manifest, destination, resolve = false) {
  const args = ['make', 'metadata-downstream', '--manifest', manifest, '--output', destination];
  if (resolve) args.push('--resolve', '1');
  const result = spawnSync('oslo', args, { encoding: 'utf8' });
  fs.writeFileSync(`${destination}.log`, result.stdout + result.stderr);
  assert.equal(result.status, 0, result.stdout + result.stderr); assert.equal(result.signal, null);
  return JSON.parse(fs.readFileSync(destination, 'utf8'));
}
const positive = metadata(`${fixture}/Cargo.toml`, `${output}/patched.json`);
const summary = { positive: verify(positive, fixture), negatives: [] };
const manifest = fs.readFileSync(`${fixture}/Cargo.toml`, 'utf8')
  .replaceAll('"../../crates/usd_bevy"', JSON.stringify(`${root}/crates/usd_bevy`))
  .replaceAll('"../../vendor/bevy_asset"', JSON.stringify(`${root}/vendor/bevy_asset`))
  .replaceAll('"../../vendor/bevy_pbr"', JSON.stringify(`${root}/vendor/bevy_pbr`));
for (const missing of ['bevy_asset', 'bevy_pbr']) {
  const workspace = `${output}/without-${missing}`; fs.mkdirSync(workspace);
  const lines = manifest.split('\n').filter(line => !line.startsWith(`${missing} =`));
  fs.writeFileSync(`${workspace}/Cargo.toml`, lines.join('\n') + `\n[lib]\npath = ${JSON.stringify(`${fixture}/src/lib.rs`)}\n`);
  fs.copyFileSync(`${fixture}/Cargo.lock`, `${workspace}/Cargo.lock`);
  const data = metadata(`${workspace}/Cargo.toml`, `${workspace}/metadata.json`, true);
  assert(one(data, missing).source.startsWith('registry+'), `${missing} negative did not resolve unpatched upstream`);
  assert.throws(() => verify(data, workspace), new RegExp(`${missing} must use the consumer's required local patch`));
  summary.negatives.push({ missing, rejected: true, resolvedSource: one(data, missing).source });
}
for (const [file, expected] of Object.entries(inputs)) assert.equal(hash(file), expected, `${file} changed`);
summary.inputs = inputs;
fs.writeFileSync(`${base}/graph-summary.json`, JSON.stringify(summary, null, 2) + '\n');
console.log(JSON.stringify(summary, null, 2));
