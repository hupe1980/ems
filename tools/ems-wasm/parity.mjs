// Compare browser exports with the daemon's native readings for the same shift.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";

const root = new URL("../../", import.meta.url);
const dependencies = execFileSync("cargo", [
  "tree", "--locked", "-p", "ems-wasm", "--target", "wasm32-unknown-unknown",
  "--edges", "normal,build", "--prefix", "none",
], { cwd: root, encoding: "utf8" });
assert.doesNotMatch(dependencies, /^\S+ v\S+ \([^\n]*[/\\]services[/\\]/m,
  "browser builds must use reusable crates");

const module = await WebAssembly.compile(await readFile(process.argv[2]));
assert.deepEqual(WebAssembly.Module.imports(module), [], "the challenge must run without host imports");
const { exports: wasm } = await WebAssembly.instantiate(module);
const receipt = {
  targetKw: wasm.target_kw(),
  batteryKw: wasm.battery_kw(),
  choices: [0, 1].map(choice => ({
    minutes: wasm.minutes_left(choice),
    deliveredKwh: wasm.delivered_kwh(choice),
    projectedKw: wasm.projected_mean_kw(choice),
  })),
  closedKwh: wasm.closed_kwh(),
  closedKw: wasm.closed_mean_kw(),
};

const native = execFileSync("cargo", [
  "run", "--quiet", "--locked", "-p", "ems-sim", "--features", "peak", "--example", "peak_receipts",
], { cwd: root, encoding: "utf8" });
assert.deepEqual(receipt, JSON.parse(native));
assert.deepEqual(receipt, {
  targetKw: 1100, batteryKw: 120,
  choices: [
    { minutes: 5, deliveredKwh: 10, projectedKw: 1100 },
    { minutes: 3, deliveredKwh: 6, projectedKw: 1116 },
  ],
  closedKwh: 285, closedKw: 1140,
});
assert.deepEqual([wasm.excess_kw(0), wasm.excess_kw(1)], [0, 16]);
for (const invalid of [-1, 2, 2147483647]) {
  for (const field of ["minutes_left", "delivered_kwh", "projected_mean_kw", "excess_kw"]) {
    assert.equal(wasm[field](invalid), -1, field);
  }
}
console.log("Peak challenge matches the native replay for both decisions and the reconciled close.");

// Independent expectations for every slider step at 230 V, 6–16 A per phase.
for (let surplus = 0; surplus <= 12000; surplus += 10) {
  for (const phases of [1, 3]) {
    const watts = surplus < 6 * 230 * phases ? 0 : Math.min(surplus, 16 * 230 * phases, 11000);
    assert.equal(wasm.solar_charge_w(surplus, phases), watts, `surplus=${surplus}, phases=${phases}`);
  }
}
for (const [surplus, phases] of [[-10, 1], [12010, 3], [11, 1], [2000, 2]]) {
  assert.equal(wasm.solar_charge_w(surplus, phases), -1);
}
console.log("Household setpoints verified for all 2,402 solar surplus and phase combinations.");
