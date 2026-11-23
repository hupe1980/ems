const controls = document.getElementById("peak-controls");
const example = document.getElementById("peak-example");
const formatter = new Intl.NumberFormat("en-GB");
const format = value => formatter.format(value);

async function start() {
  const response = await fetch(new URL("../ems.wasm", import.meta.url));
  if (!response.ok) throw new Error("Peak model unavailable");
  const { instance } = await WebAssembly.instantiate(await response.arrayBuffer());
  const model = instance.exports;
  const buttons = [...controls.querySelectorAll("[data-choice]")];
  const reveal = document.getElementById("peak-reveal");
  const disclosure = reveal.querySelector("button");
  const meter = document.getElementById("peak-meter");

  // Read both supported choices before replacing the static example.
  const outcomes = buttons.map(button => {
    const choice = Number(button.dataset.choice);
    return {
      minutes: model.minutes_left(choice),
      power: model.battery_kw(),
      energy: model.delivered_kwh(choice),
      mean: model.projected_mean_kw(choice),
      excess: model.excess_kw(choice),
    };
  });
  const target = model.target_kw();
  const closedEnergy = model.closed_kwh();
  const closedMean = model.closed_mean_kw();
  const values = [target, closedEnergy, closedMean, ...outcomes.flatMap(Object.values)];
  if (!values.every(value => Number.isFinite(value) && value >= 0) ||
      outcomes.some(({ minutes }) => !Number.isInteger(minutes) || minutes < 1 || minutes > 5)) {
    throw new Error("Peak model returned an invalid reading");
  }
  document.getElementById("peak-target").textContent = format(target) + " kW";
  document.getElementById("peak-capacity").textContent = format(outcomes[0].power) + " kW";
  for (const [index, button] of buttons.entries()) {
    button.addEventListener("click", () => {
      const result = outcomes[index];
      for (const option of buttons) option.setAttribute("aria-pressed", String(button === option));
      for (const mark of controls.querySelectorAll("[data-minute]")) {
        mark.classList.toggle("supplying", Number(mark.dataset.minute) <= result.minutes);
      }
      document.getElementById("peak-mean").textContent = format(result.mean) + " kW";
      const verdict = document.getElementById("peak-verdict");
      verdict.textContent = result.excess > 0 ? format(result.excess) + " kW over target" : "Target met";
      verdict.className = result.excess > 0 ? "peak-over" : "peak-met";
      document.getElementById("peak-equation").textContent =
        format(result.power) + " kW for " + result.minutes + " minutes delivers " + format(result.energy) + " kWh.";
      reveal.hidden = false;
    });
  }
  disclosure.addEventListener("click", () => {
    const opening = meter.hidden;
    disclosure.setAttribute("aria-expanded", String(opening));
    disclosure.textContent = opening ? "Hide the meter close" : "Reveal the meter close";
    meter.textContent = opening ? "With the battery idle, the original meter recorded " +
      format(closedEnergy) + " kWh over 15 minutes, an average of " + format(closedMean) + " kW. Over 15 minutes, each 1 kWh of avoided import lowers the mean by 4 kW." : "";
    meter.hidden = !opening;
  });
  // A visitor may have opened the worked example during a slow download.
  if (!example.open) example.hidden = true;
  controls.hidden = false;
}

start().catch(() => {
  example.open = true;
  document.getElementById("peak-unavailable").hidden = false;
});
