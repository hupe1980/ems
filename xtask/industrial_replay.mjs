// Exercise the same read-only rehearsal a shift engineer opens on the gateway.
export async function verifyIndustrialReplay(base, send, js) {
  const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
  async function visible(text, hash = "", { focused = false } = {}) {
    for (let i = 0; i < 100; i++) {
      const ready = await js(`document.readyState === "complete" && (!${focused} || document.activeElement === document.querySelector(":target")) && location.origin === ${JSON.stringify(base)} && location.hash === ${JSON.stringify(hash)} && document.body.innerText.includes(${JSON.stringify(text)}) && document.documentElement.scrollWidth <= innerWidth`);
      if (ready) return;
      await wait(30);
    }
    throw Error(`Industrial replay expected "${text}" at ${await js("location.href")} (actual page: ${await js("document.body.innerText")})`);
  }
  async function key(key, code, windowsVirtualKeyCode) {
    for (const type of ["keyDown", "keyUp"]) {
      await send("Input.dispatchKeyEvent", {type, key, code, windowsVirtualKeyCode});
    }
  }
  async function activate(text) {
    for (let i = 0; i < 16; i++) {
      if (await js(`document.activeElement?.matches("a") && document.activeElement.textContent === ${JSON.stringify(text)}`)) {
        await key("Enter", "Enter", 13);
        return;
      }
      await key("Tab", "Tab", 9);
    }
    throw Error("Cannot reach industrial decision by keyboard: " + text);
  }
  await send("Emulation.setDeviceMetricsOverride", {width:320,height:720,deviceScaleFactor:1,mobile:true});
  await send("Page.navigate", {url:base+"/?moment=early"});
  await visible("1100 kW mit Batterie");
  await activate("Jetzt starten");
  await visible("1100 kW mit Batterie", "#entscheidung");
  await activate("Zählerabschluss aufdecken");
  await visible("1140 kW im abgeschlossenen Intervall", "#abschluss");
  await send("Page.navigate", {url:base+"/?moment=early"});
  await visible("1100 kW mit Batterie");
  await activate("Zwei Minuten warten");
  await visible("1116 kW mit Batterie", "#entscheidung", { focused: true });
  if (await js('!!document.querySelector("#abschluss")')) throw Error("Decision disclosed a future close");
  await activate("Zählerabschluss aufdecken");
  await visible("1140 kW im abgeschlossenen Intervall", "#abschluss", { focused: true });
  if (!await js('document.body.innerText.includes("1116 kW mit Batterie")')) throw Error("Reveal lost the selected branch");
  await send("Page.reload");
  await visible("1140 kW im abgeschlossenen Intervall", "#abschluss");
  await js("history.back()");
  await visible("1116 kW mit Batterie", "#entscheidung");
  await js('document.querySelector("details summary").focus()');
  await key(" ", "Space", 32);
  await visible("keine verbleibende Startminute", "#entscheidung");
  await send("Page.navigate", {url:base+"/?moment=delayed&evidence=missing-close&reveal=true"});
  await visible("Zählerabschluss ausstehend");
  const api = await (await fetch(base+"/v1/demo/industrial?moment=delayed&reveal=true")).json();
  if (api.closed_kwh !== 285 || api.closed_mean_kw !== 1140 || api.residual_peak_kw !== 1116 || api.deliverable_ac_kwh !== 6) {
    throw Error("Industrial reveal disagrees with the fixture: "+JSON.stringify(api));
  }
}
