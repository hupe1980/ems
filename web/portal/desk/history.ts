import uPlot from "uplot";
import "./style.css";

interface Reading {
	start_unix_s: number;
	import_kwh: string;
	export_kwh: string;
}
interface WindowData {
	first_day: string;
	day: string;
	start_unix_s: number;
	end_unix_s: number;
	closed_until_unix_s: number;
	intervals: Reading[];
}

const zone = "Europe/Berlin";
const date = new Intl.DateTimeFormat("sv-SE", { timeZone: zone });
const shortDate = new Intl.DateTimeFormat("en-GB", {
	day: "numeric",
	month: "short",
	timeZone: zone,
});
const clock = new Intl.DateTimeFormat("en-GB", {
	hour: "2-digit",
	minute: "2-digit",
	timeZone: zone,
});
const stamp = new Intl.DateTimeFormat("en-GB", {
	day: "numeric",
	month: "short",
	year: "numeric",
	hour: "2-digit",
	minute: "2-digit",
	timeZoneName: "shortOffset",
	timeZone: zone,
});
const number = new Intl.NumberFormat("en-GB");

interface Inspection {
	at: number;
	min: number;
	max: number;
}

// The historian normalizes decimal strings. Rank exact values before converting to canvas coordinates.
export function greaterEnergy(a: string, b: string) {
	const integerDigits = (s: string) => s.split(".")[0].length;
	return integerDigits(a) === integerDigits(b)
		? a > b
		: integerDigits(a) > integerDigits(b);
}

class MeterHistory extends HTMLElement {
	private plot?: uPlot;
	private observer?: ResizeObserver;

	disconnectedCallback() {
		this.observer?.disconnect();
		this.plot?.destroy();
		this.plot = undefined;
	}

	show(
		window: WindowData,
		openDay: (day: string, inspection: Inspection) => void,
		restore?: Inspection,
	) {
		const start = window.start_unix_s;
		const count = (window.end_unix_s - start) / 900;
		const closed = (window.closed_until_unix_s - start) / 900;
		const x = Array.from({ length: count }, (_, i) => start + i * 900);
		const points = new Map(window.intervals.map((q) => [q.start_unix_s, q]));
		const imports: (number | null)[] = [],
			exports: (number | null)[] = [];
		let largest = "0";
		let measured = 0,
			invalid = 0,
			peak = -1,
			firstGap = -1;
		for (const [i, t] of x.entries()) {
			const q = points.get(t);
			const valid = q && Number(q.import_kwh) >= 0 && Number(q.export_kwh) >= 0;
			imports.push(valid ? Number(q.import_kwh) : null);
			exports.push(valid ? -Number(q.export_kwh) : null);
			if (valid) {
				measured++;
				if (greaterEnergy(q.import_kwh, largest)) {
					peak = i;
					largest = q.import_kwh;
				}
			} else if (q) invalid++;
			else if (i < closed && firstGap < 0) firstGap = i;
		}
		this.innerHTML = `
            <header class="range-header"><div><h3>Grid exchange</h3><p class="coverage"></p></div>
            <div class="legend"><span>Import ↑ · kWh</span><span>Export ↓ · kWh</span></div></header>
            <p class="history-hint">Drag to zoom. Tap or use the arrow keys to inspect a quarter.</p>
            <div class="history-chart" tabindex="0" role="slider" aria-label="Inspect grid exchange by quarter hour"></div>
            <div class="sample"><time></time><div><strong></strong><span></span></div></div>
            <div class="history-actions">
            <button type="button" data-action="peak">Peak import</button>
            <button type="button" data-action="gap">First gap</button>
            <button type="button" data-action="zoom">Zoom to day</button>
            <button type="button" data-action="reset">Reset zoom</button>
            <button type="button" data-action="day">Open meter day →</button></div>`;
		const find = <E extends HTMLElement>(selector: string) => {
			const node = this.querySelector<E>(selector);
			if (!node) throw new Error(`Missing meter history element: ${selector}`);
			return node;
		};
		const area = find<HTMLDivElement>(".history-chart");
		const time = find<HTMLTimeElement>("time"),
			value = find("strong"),
			second = find(".sample span");
		const buttons = (name: string) =>
			find<HTMLButtonElement>(`[data-action="${name}"]`);
		const missing = closed - measured - invalid;
		find(".coverage").textContent = [
			shortDate.format(start * 1000) +
				" – " +
				shortDate.format((window.end_unix_s - 1) * 1000),
			number.format(measured) +
				" of " +
				number.format(closed) +
				" closed quarters measured",
			missing && `${number.format(missing)} missing`,
			invalid && `${number.format(invalid)} invalid`,
			count > closed && `${count - closed} open`,
		]
			.filter(Boolean)
			.join(" · ");
		const initial = restore
			? (restore.at - start) / 900
			: Math.max(0, closed - 1);
		let selected = initial;
		function inspect(index: number) {
			selected = Math.max(0, Math.min(count - 1, index));
			const t = x[selected],
				q = points.get(t);
			time.dateTime = new Date(t * 1000).toISOString();
			time.textContent = stamp.format(t * 1000);
			value.textContent = q
				? (imports[selected] === null ? "Invalid · " : "") +
					"Import " +
					q.import_kwh +
					" kWh"
				: selected < closed
					? "Missing reading"
					: "Quarter still open";
			second.textContent = q ? `Export ${q.export_kwh} kWh` : "";
			area.setAttribute("aria-valuenow", String(selected + 1));
			area.setAttribute(
				"aria-valuetext",
				[time.textContent, value.textContent, second.textContent]
					.filter(Boolean)
					.join(" · "),
			);
		}
		area.setAttribute("aria-valuemin", "1");
		area.setAttribute("aria-valuemax", String(count));
		const bar = uPlot.paths.bars?.({ size: [0.92, 24] });
		if (!bar)
			throw new Error("The desk bundle requires the uPlot bar renderer");
		const domain = { min: start - 450, max: x[count - 1] + 450 };
		// Keep the entire window on screen even when the first and last days have no data.
		const plot = new uPlot(
			{
				width: Math.max(1, area.clientWidth),
				height: 240,
				tzDate: (s) => uPlot.tzDate(new Date(s * 1000), zone),
				scales: {
					x: { range: (_, min, max) => [min, max] },
					y: {
						range: (_, lo, hi) => {
							const extent =
								Math.max(Math.abs(lo ?? 0), Math.abs(hi ?? 0)) || 1;
							return [-extent * 1.08, extent * 1.08];
						},
					},
				},
				legend: { show: false },
				cursor: {
					drag: { x: true, y: false, dist: 8 },
					y: false,
					focus: { prox: -1 },
				},
				axes: [
					{
						stroke: "#536c63",
						grid: { show: false },
						space: 80,
						values: (u, ticks) =>
							ticks.map((t) =>
								((u.scales.x.max ?? domain.max) -
									(u.scales.x.min ?? domain.min) <=
								90000
									? clock
									: shortDate
								).format(t * 1000),
							),
					},
					{
						stroke: "#536c63",
						grid: { stroke: "#dce6df", width: 1 },
						size: 56,
						values: (_, ticks) => ticks.map((v) => number.format(Math.abs(v))),
					},
				],
				series: [
					{},
					{
						label: "Import (kWh)",
						stroke: "#116751",
						fill: "#11775e",
						paths: bar,
						points: { show: false },
					},
					{
						label: "Export (kWh)",
						stroke: "#8c5013",
						fill: "#c17a2c",
						paths: bar,
						points: { show: false },
					},
				],
				hooks: {
					setCursor: [
						(u) => {
							if (u.cursor.idx != null) inspect(u.cursor.idx);
						},
					],
					setScale: [
						(u, key) => {
							if (key === "x")
								buttons("reset").disabled =
									u.scales.x.min === domain.min &&
									u.scales.x.max === domain.max;
						},
					],
				},
			},
			[x, imports, exports],
			area,
		);
		this.plot = plot;
		const setRange = (range: { min: number; max: number }) =>
			plot.batch(() => plot.setScale("x", range));
		setRange(restore ?? domain);
		const reset = () => setRange(domain);
		function move(i: number) {
			const index = Math.max(0, Math.min(count - 1, i));
			if (
				x[index] < (plot.scales.x.min ?? domain.min) ||
				x[index] > (plot.scales.x.max ?? domain.max)
			)
				reset();
			plot.setCursor(
				{
					left: plot.valToPos(x[index], "x"),
					top: plot.valToPos(0, "y"),
				},
				false,
			);
			inspect(index);
		}
		// A horizontal touch inspects the nearest quarter. Vertical swipes scroll the page.
		const scrub = (event: PointerEvent) => {
			if (event.pointerType !== "touch" || event.buttons === 0) return;
			const left = event.clientX - plot.over.getBoundingClientRect().left;
			move(plot.posToIdx(Math.min(plot.over.clientWidth, Math.max(0, left))));
		};
		plot.over.onpointerdown = scrub;
		plot.over.onpointermove = scrub;
		buttons("peak").disabled = peak < 0;
		buttons("gap").disabled = firstGap < 0;
		buttons("peak").onclick = () => {
			move(peak);
			area.focus();
		};
		buttons("gap").onclick = () => {
			move(firstGap);
			area.focus();
		};
		buttons("reset").onclick = () => {
			const index = selected;
			reset();
			move(index);
			area.focus();
		};
		buttons("zoom").onclick = () => {
			const day = date.format(x[selected] * 1000);
			let a = selected,
				b = selected;
			while (a > 0 && date.format(x[a - 1] * 1000) === day) a--;
			while (b < count - 1 && date.format(x[b + 1] * 1000) === day) b++;
			const index = selected;
			setRange({ min: x[a] - 450, max: x[b] + 450 });
			move(index);
			area.focus();
		};
		buttons("day").onclick = () =>
			openDay(date.format(x[selected] * 1000), {
				at: x[selected],
				min: plot.scales.x.min ?? domain.min,
				max: plot.scales.x.max ?? domain.max,
			});
		area.onkeydown = (e) => {
			const target: Record<string, number> = {
				ArrowRight: selected + 1,
				ArrowLeft: selected - 1,
				Home: Math.ceil(((plot.scales.x.min ?? domain.min) - start) / 900),
				End: Math.floor(((plot.scales.x.max ?? domain.max) - start) / 900),
			};
			if (e.key in target) {
				e.preventDefault();
				move(target[e.key]);
			}
		};
		this.observer = new ResizeObserver(() => {
			if (area.clientWidth > 0 && plot.width !== area.clientWidth) {
				const index = selected;
				plot.batch(() =>
					plot.setSize({ width: area.clientWidth, height: 240 }),
				);
				move(index);
			}
		});
		this.observer.observe(area);
		move(initial);
		if (restore) area.focus();
	}
}
customElements.define("meter-history", MeterHistory);
