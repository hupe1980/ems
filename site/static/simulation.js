// Both simulations on a page share one instance of the Rust model.
async function loadModel() {
  const response = await fetch(new URL("./ems.wasm", import.meta.url));
  if (!response.ok) throw new Error("Simulation unavailable");
  const { instance } = await WebAssembly.instantiate(await response.arrayBuffer());
  return instance.exports;
}

export const modelExports = loadModel();
