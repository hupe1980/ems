import { modelExports } from "../simulation.js";

const controls = document.querySelector("#solar-controls");
const slider = document.querySelector("#solar-surplus");
const example = document.querySelector("#solar-example");
const cards = [...controls.querySelectorAll(".solar-modes section")];
const format = watts => (watts / 1000).toFixed(2) + " kW";

function fallback() {
  controls.hidden = true;
  example.hidden = false;
  example.open = true;
  document.querySelector("#solar-failed").hidden = false;
}

async function start() {
  const charge = (await modelExports).solar_charge_w;
  function update() {
    const surplus = slider.valueAsNumber;
    const results = [charge(surplus, 1), charge(surplus, 3)];
    if (!results.every(w => Number.isInteger(w) && w >= 0 && w <= surplus)) {
      throw new Error("Invalid charging setpoint");
    }
    document.querySelector("#solar-available").value = format(surplus);
    slider.setAttribute("aria-valuetext", format(surplus));
    results.forEach((watts, i) => {
      const card = cards[i];
      card.querySelector(".solar-reading").textContent = watts ? format(watts) : "Paused";
      card.querySelector("meter").value = watts;
      card.querySelector(".solar-reason").textContent = watts
        ? (watts === surplus ? "Uses all available surplus." : format(surplus - watts) + " remains available.")
        : "Needs at least " + (i === 0 ? "1.38" : "4.14") + " kW to start.";
    });
  }
  update();
  slider.addEventListener("input", () => { try { update(); } catch { fallback(); } });
  for (const preset of controls.querySelectorAll("[data-watts]")) {
    preset.addEventListener("click", () => {
      slider.value = preset.dataset.watts;
      slider.dispatchEvent(new Event("input"));
    });
  }
  controls.hidden = false;
  if (!example.open) example.hidden = true;
}
start().catch(fallback);
