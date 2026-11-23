# EMS browser simulations

The Zola page asks when to start a 120 kW battery. At build time, `ems_sim::peak` evaluates the bundled trace used by `cemsd demo`. The browser selects a precomputed result. Its inputs are two choices, encoded as 0 (start now) and 1 (wait two minutes). The original meter close is independent of that choice.

The household solar simulator compares the steady-state setpoints of a switchable wallbox. The build runs `ems_device::realisable` for every selectable surplus (0–12,000 W in 10 W steps) and both phase modes. Those 2,402 results are embedded in the same module. The page needs no device connection.

From the workspace root, run `just site-build` to compile the module, compare its answers with the native replay and build the site. Preview with `just site` and open `/solar-charging/` or `/peak-challenge/`. To exercise the UI and static fallback, run `uv run xtask/peak_challenge.py` with Chrome installed. Set `CHROME` to its executable on Linux.

Change the dispatch calculation in [ems-sim](../../crates/ems-sim/src/peak/mod.rs). The [browser adapter](src/lib.rs) exposes numbers, and the [page script](../../site/static/peak/challenge.js) renders them. Update the [worked example](../../site/content/peak-challenge.md) if the reference shift changes.
