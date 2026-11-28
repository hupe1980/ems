"use strict";
const en = {
  linkType:"Connection", serialBus:"Serial bus", title:"Set up the grid connection", code:"Gateway access code", sign:"Sign in", role:"Who is setting up?", resident:"Household", installer:"Installer", handover:"After commissioning, open the handover sheet in Setup.", contact:"Support contact (optional)", connection:"Grid connection", phases:"I confirm this is a three-phase connection", fuse:"Main fuse per phase (A)", device:"Connection meter", meterHint:"Match the reported devices to their nameplates.", address:"IP address", tariff:"Fixed tariff", energy:"Net energy price (ct/kWh)", network:"Net network charge (ct/kWh)", standing:"Standing charge (€/year)", roof:"Observe solar PV", storage:"Observe battery", roofSize:"Installed array (kWp)", storageSize:"Usable capacity (kWh)", power:"AC nameplate power (kW)", date:"Commissioning date on installation sheet", observe:"Observation mode", authorityHint:"The gateway reads solar output and battery state of charge. The independently commissioned controller continues to enforce the grid limits.", authority:"Responsible controller on installation sheet", review:"Review devices", apply:"Install and restart gateway", cancel:"Back"
};
const words = [...document.querySelectorAll("[data-word]")];
const de = Object.fromEntries(words.map(el => [el.dataset.word, el.textContent]));
let english = (new URLSearchParams(location.search).get("lang") || navigator.language).startsWith("en");
const el = (id) => document.getElementById(id);
const form = el("meter");
let proposal = null;
function language() {
  document.documentElement.lang = english ? "en" : "de";
  const labels = english ? en : de;
  words.forEach(node => {node.textContent = labels[node.dataset.word];});
  document.title = labels.title;
  el("language").textContent = english ? "Deutsch" : "English";
}
el("language").onclick = () => { english = !english; language(); };
language();
function say(text) { el("status").textContent = text; }
async function request(path, body) {
  const opts = body == null ? {} : {method:"POST", headers:{"content-type":"application/json"}, body:JSON.stringify(body)};
  const result = await fetch(path, opts);
  if (!result.ok) {
    if (result.status === 401) {el("login").hidden = false; form.hidden = true; el("review").hidden = true;}
    const refusal = await result.json().catch(() => null);
    throw new Error(refusal?.detail || result.statusText);
  }
  return result.status === 204 ? null : result.json();
}
async function ready() {
  const receipt = await request("/v1/commissioning/change");
  const buses = await request("/v1/commissioning/serial-buses");
  form.elements.bus.replaceChildren(...buses.map(name => new Option(name, name)));
  el("meter-connection").hidden = !buses.length;
  el("login").hidden = true; form.hidden = false;
  if (receipt?.outcome === "restored") say(english ? "Previous configuration restored. Check the selected devices, connections and Modbus units." : "Vorherige Konfiguration wiederhergestellt. Gewählte Geräte, Verbindungen und Modbus-Units prüfen.");
}
el("login").onsubmit = async event => {
  event.preventDefault();
  try {
    await request("/v1/session", {token:event.target.elements.token.value.trim()});
    event.target.reset(); say(""); await ready();
  } catch (error) {say(error.message);}
};
function endpoint(host, port) {
  const ip = host.trim();
  return (ip.includes(":") && !ip.startsWith("[") ? "[" + ip + "]" : ip) + ":" + port;
}
function selectedAsset(key) {
  const f = form.elements;
  if (!f[key].checked) return null;
  return {address:endpoint(f[key+"_host"].value, f[key+"_port"].value), unit:f[key+"_unit"].valueAsNumber, size:f[key+"_size"].valueAsNumber, power_kw:f[key+"_power"].valueAsNumber, commissioned:f[key+"_date"].value};
}
form.onchange = () => {
  const f = form.elements;
  const serial = f.link.value === "serial";
  el("meter-tcp").hidden = f.host.disabled = f.port.disabled = serial;
  el("meter-bus").hidden = f.bus.disabled = !serial;
  f.unit.min = serial ? 1 : 0; f.unit.max = serial ? 247 : 255;
  el("handover").hidden = f.role.value !== "installer";
  for (const key of ["roof", "storage"]) {
    const group = el(key+"-fields");
    if (group.disabled && f[key].checked && !f[key+"_host"].value) {
      f[key+"_host"].value = f.host.value;
      f[key+"_port"].value = f.port.value;
    }
    group.hidden = group.disabled = !f[key].checked;
  }
  el("authority").hidden = el("authority").disabled = !(f.roof.checked || f.storage.checked);
};
form.onsubmit = async event => {
  event.preventDefault();
  const submit = form.querySelector("button");
  submit.disabled = true;
  const f = form.elements, host = f.host.value.trim();
  try {
    const r = await request("/v1/commissioning/first-meter", {
      ...(f.link.value === "serial" ? {serial_bus:f.bus.value} : {address:endpoint(host, f.port.value)}), roof:selectedAsset("roof"), storage:selectedAsset("storage"), external_limit_controller:el("authority").disabled ? null : f.controller.value.trim(),
      unit:f.unit.valueAsNumber, phases:f.phases.checked ? 3 : 0, fuse_a:f.fuse.valueAsNumber,
      energy_ct:f.energy.valueAsNumber, network_ct:f.network.valueAsNumber, standing_eur:f.standing.valueAsNumber, support:f.support.value.trim()
    });
    proposal = r.proposal; say(""); form.hidden = true; el("review").hidden = false;
    el("device").textContent = r.device; el("block").textContent = r.toml; el("device").focus();
  } catch (error) {say(error.message);}
  finally {submit.disabled = false;}
};
el("cancel").onclick = () => {proposal = null; el("review").hidden = true; form.hidden = false; form.querySelector("button").focus();};
el("apply").onclick = async () => {
  if (!proposal) return;
  el("apply").disabled = true;
  try {
    await request("/v1/commissioning/change", {id:proposal});
  } catch (error) {
    if (!(error instanceof TypeError)) {say(error.message); el("apply").disabled = false; return;}
  }
  say(english ? "Waiting for device readings…" : "Gerätemessungen ausstehend…");
  const deadline = Date.now() + 200000;
  const poll = async () => {
    try {
      const receipt = await request("/v1/commissioning/change");
      if (receipt?.id === proposal && receipt.outcome !== "restarting") {
        history.replaceState(null, "", "/?lang=" + (english ? "en" : "de") + "#setup");
        location.reload();
        return;
      }
    } catch { /* listener is restarting */ }
    if (Date.now() > deadline) {
      say(english ? "Gateway unavailable. Check its connection, then retry this confirmation." : "Gateway nicht erreichbar. Verbindung prüfen und Bestätigung erneut senden.");
      el("apply").disabled = false;
      return;
    }
    setTimeout(poll, 1000);
  };
  poll();
};
ready().catch(() => {});
