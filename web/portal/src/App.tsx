import { useCallback, useEffect, useRef, useState } from "react";

import {
	downloadReceipt,
	type Receipt,
	readReceipt,
	SessionEnded,
} from "./receipt";

const energy = new Intl.NumberFormat("en-GB", {
	minimumFractionDigits: 2,
	maximumFractionDigits: 3,
});
const kwh = (wh: number) => energy.format(wh / 1000);
const register = new Intl.NumberFormat("en-GB");
const localDeparture = new Intl.DateTimeFormat("en-GB", {
	day: "numeric",
	month: "short",
	year: "numeric",
	hour: "2-digit",
	minute: "2-digit",
	timeZoneName: "short",
	timeZone: "Europe/Berlin",
});

function Evidence({ receipt }: { receipt: Receipt }) {
	const { evidence: e, delivered_wh: delivered, shortfall_wh: short } = receipt;
	return (
		<section className="receipt" aria-labelledby="outcome">
			<header className="meta">
				<span>Simulated departure</span>
				<span>{receipt.site}</span>
			</header>
			<h2 id="outcome" tabIndex={-1}>
				{short === null
					? "Departure unverified"
					: short === 0
						? "Charging goal met"
						: `${kwh(short)} kWh short`}
			</h2>
			<p>
				{delivered === null
					? `Last confirmed at reconnection: ${kwh(e.rejoin_wh - e.opening_wh)} kWh. The final outlet reading is missing.`
					: `${kwh(delivered)} of ${kwh(e.requested_wh)} kWh delivered to the outlet.`}
			</p>
			{delivered !== null && (
				<meter
					min={0}
					max={e.requested_wh}
					value={Math.min(delivered, e.requested_wh)}
					aria-label="Requested energy delivered"
				/>
			)}
			<dl className="facts">
				<div>
					<dt>Departure</dt>
					<dd>
						<time dateTime={e.departure}>
							{localDeparture.format(new Date(e.departure))}
						</time>
					</dd>
				</div>
				<div>
					<dt>Requested</dt>
					<dd>{kwh(e.requested_wh)} kWh</dd>
				</div>
			</dl>
			<details>
				<summary>Outlet meter readings</summary>
				<dl className="readings">
					{(
						[
							["At plug-in", e.opening_wh],
							["At reconnection", e.rejoin_wh],
							["At departure", e.closing_wh],
						] as const
					).map(([label, value]) => (
						<div key={label}>
							<dt>{label}</dt>
							<dd>
								{value === null ? "Missing" : `${register.format(value)} Wh`}
							</dd>
						</div>
					))}
				</dl>
			</details>
		</section>
	);
}

interface Day {
	day: string;
	imported: number;
	exported: number;
	produced: number;
}
type Flow = "imported" | "exported" | "produced";
const flows: Record<Flow, string> = {
	imported: "Grid import",
	exported: "Grid export",
	produced: "Generation",
};
const shortDate = new Intl.DateTimeFormat("en-GB", {
	day: "numeric",
	month: "short",
	year: "numeric",
	timeZone: "UTC",
});
const dayLabel = (day: string) =>
	shortDate.format(new Date(`${day}T00:00:00Z`));
function History({ days }: { days: Day[] }) {
	const [flow, setFlow] = useState<Flow>("imported");
	const [windowDays, setWindowDays] = useState(7);
	const [selectedDay, setSelectedDay] = useState<string | null>(null);
	if (!days.length)
		return (
			<section>
				<h2>Energy history</h2>
				<p>No daily energy reports have arrived.</p>
			</section>
		);
	const end = Date.parse(days[0].day);
	const byDate = new Map(days.map((d) => [d.day, d]));
	const dates = Array.from({ length: windowDays }, (_, i) =>
		new Date(end - (windowDays - 1 - i) * 86400000).toISOString().slice(0, 10),
	);
	const reported = days.filter((d) => d.day >= dates[0]);
	const peak = Math.max(...reported.map((d) => d[flow]));
	const scaleMax = Math.max(
		...reported.flatMap((d) => [d.imported, d.exported, d.produced]),
	);
	const total = reported.reduce((sum, d) => sum + d[flow], 0);
	const selected =
		selectedDay && dates.includes(selectedDay) ? selectedDay : days[0].day;
	const latestGap = [...dates].reverse().find((date) => !byDate.has(date));
	const highest = reported.find((day) => peak > 0 && day[flow] === peak);
	const selectedReading = byDate.get(selected);
	const readingText = selectedReading
		? `${energy.format(selectedReading[flow])} kWh`
		: "No report";
	return (
		<section className="journal">
			<header className="controls">
				<h2>Energy history</h2>
				<select
					aria-label="History period"
					value={windowDays}
					onChange={(e) => setWindowDays(Number(e.target.value))}
				>
					<option value={7}>7 days</option>
					<option value={30}>30 days</option>
				</select>
			</header>
			<fieldset className="metrics">
				<legend className="visually-hidden">Energy measure</legend>
				{(Object.keys(flows) as Flow[]).map((key) => (
					<label key={key}>
						<input
							type="radio"
							name="measure"
							checked={flow === key}
							onChange={() => setFlow(key)}
						/>
						<span>{flows[key]}</span>
					</label>
				))}
			</fieldset>
			<div aria-live="polite">
				<strong className="energy-total">{energy.format(total)} kWh</strong>
				<p>
					{flows[flow]} · {reported.length} of {windowDays} days reported.
				</p>
			</div>
			<div className="plot-limit">
				Shared scale · {energy.format(scaleMax)} kWh
			</div>
			<div className="day-plot">
				<svg
					className="bars"
					aria-hidden="true"
					viewBox={`0 0 ${windowDays * 10} 100`}
					preserveAspectRatio="none"
				>
					{dates.map((date, i) => {
						const report = byDate.get(date);
						const h = report && scaleMax ? (100 * report[flow]) / scaleMax : 0;
						return (
							<g key={date}>
								{date === selected && (
									<rect x={i * 10} width="10" height="100" fill="#d6e7dd" />
								)}
								{report ? (
									<rect
										x={i * 10 + 1}
										width="8"
										y={h === 0 ? 99 : 100 - h}
										height={h === 0 ? 1 : h}
										fill={date === selected ? "#164d39" : "#538772"}
									/>
								) : (
									<path
										d={`M${i * 10 + 2},96 l6,-6`}
										stroke="#627168"
										vectorEffect="non-scaling-stroke"
									/>
								)}
							</g>
						);
					})}
				</svg>
				<input
					type="range"
					aria-label="Inspect energy by day"
					aria-valuetext={`${dayLabel(selected)}, ${flows[flow]}, ${readingText}`}
					min="0"
					max={windowDays - 1}
					value={dates.indexOf(selected)}
					onChange={(e) => setSelectedDay(dates[Number(e.target.value)])}
				/>
			</div>
			<div className="axis">
				<span>{dayLabel(dates[0])}</span>
				<span>{dayLabel(days[0].day)}</span>
			</div>
			<div className="day-reading">
				<button
					type="button"
					aria-label="Previous day"
					disabled={selected === dates[0]}
					onClick={() => setSelectedDay(dates[dates.indexOf(selected) - 1])}
				>
					‹
				</button>
				<div aria-live="polite">
					<time dateTime={selected}>{dayLabel(selected)}</time>
					<strong>{readingText}</strong>
				</div>
				<button
					type="button"
					aria-label="Next day"
					disabled={selected === days[0].day}
					onClick={() => setSelectedDay(dates[dates.indexOf(selected) + 1])}
				>
					›
				</button>
			</div>
			<div className="history-shortcuts">
				{highest && (
					<button type="button" onClick={() => setSelectedDay(highest.day)}>
						Highest {flows[flow].toLowerCase()}
					</button>
				)}
				{latestGap && (
					<button type="button" onClick={() => setSelectedDay(latestGap)}>
						Latest missing day
					</button>
				)}
			</div>
			<details>
				<summary>Daily readings</summary>
				<table>
					<thead>
						<tr>
							<th>Date</th>
							<th>Import</th>
							<th>Export</th>
							<th>Generation</th>
						</tr>
					</thead>
					<tbody>
						{[...dates].reverse().map((date) => {
							const d = byDate.get(date);
							return (
								<tr key={date}>
									<th>{date}</th>
									{!d ? (
										<td colSpan={3}>No report</td>
									) : (
										Object.keys(flows).map((key) => (
											<td key={key}>{energy.format(d[key as Flow])}</td>
										))
									)}
								</tr>
							);
						})}
					</tbody>
					<caption>Reported energy in kWh</caption>
				</table>
			</details>
		</section>
	);
}
async function readHistory(signal: AbortSignal): Promise<Day[]> {
	const response = await fetch("/v1/me/history", {
		signal,
		cache: "no-store",
		redirect: "error",
	});
	if (response.status === 401 || response.status === 403)
		throw new SessionEnded();
	if (!response.ok) throw new Error("Energy history is unavailable.");
	const rows: Day[] = await response.json();
	if (
		!Array.isArray(rows) ||
		rows.some(
			(d) =>
				typeof d.day !== "string" ||
				!/^\d{4}-\d{2}-\d{2}$/.test(d.day) ||
				!Number.isFinite(Date.parse(d.day)) ||
				new Date(d.day).toISOString().slice(0, 10) !== d.day ||
				[d.imported, d.exported, d.produced].some(
					(v) => !Number.isFinite(v) || v < 0,
				),
		)
	)
		throw Error("Energy history is unreadable.");
	return rows.sort((a, b) => b.day.localeCompare(a.day));
}
export function App() {
	const [days, setDays] = useState<Day[] | null>(null),
		[receipt, setReceipt] = useState<Receipt | null>(null),
		[message, setMessage] = useState("Loading household…"),
		[receiptReady, setReceiptReady] = useState(false),
		[sessionEnded, setSessionEnded] = useState(false);
	const req = useRef<AbortController | null>(null);
	const load = useCallback(async (download = false) => {
		req.current?.abort();
		const controller = new AbortController();
		req.current = controller;
		setMessage("Loading household…");
		// A revoked grant clears both panels as soon as either endpoint reports it.
		function unavailable(error: unknown): undefined {
			if (error instanceof SessionEnded && !controller.signal.aborted) {
				controller.abort();
				setDays(null);
				setReceipt(null);
				setReceiptReady(false);
				setSessionEnded(true);
				setMessage(error.message);
			}
			return undefined;
		}
		const [history, departure] = await Promise.all([
			readHistory(controller.signal).catch(unavailable),
			readReceipt(controller.signal).catch(unavailable),
		]);
		if (controller.signal.aborted) return;
		setSessionEnded(false);
		setDays(history ?? null);
		setReceipt(departure ?? null);
		setReceiptReady(departure !== undefined);
		const notices: string[] = [];
		if (!history)
			notices.push("Energy history is unavailable. Try refreshing.");
		if (departure === undefined)
			notices.push("The departure receipt is unavailable. Try refreshing.");
		if (download && departure !== undefined) {
			if (departure) {
				downloadReceipt(departure);
				notices.push("Receipt downloaded.");
			} else notices.push("There is no departure receipt to download.");
		}
		setMessage(notices.join(" "));
	}, []);
	useEffect(() => {
		void load();
		const leave = () => {
			req.current?.abort();
			setDays(null);
			setReceipt(null);
		};
		const restore = (e: PageTransitionEvent) => {
			if (e.persisted) void load();
		};
		window.addEventListener("pagehide", leave);
		window.addEventListener("pageshow", restore);
		return () => {
			req.current?.abort();
			window.removeEventListener("pagehide", leave);
			window.removeEventListener("pageshow", restore);
		};
	}, [load]);
	return (
		<main>
			<div className="home-banner">
				<header className="mast">
					<b>EMS</b>
					<span>Household portal</span>
					<a href="/oauth2/sign_out">Sign out</a>
				</header>
				<header className="controls">
					<h1>Home energy</h1>
					<button type="button" onClick={() => void load()}>
						Refresh
					</button>
				</header>
				{receipt && days && days.length > 0 && (
					<a href="#outcome">View departure receipt</a>
				)}
			</div>
			<p role="status">{message}</p>
			<div className="portal-panels">
				{days !== null && <History days={days} />}
				{receipt ? (
					<div>
						<Evidence receipt={receipt} />
						<button
							type="button"
							className="download"
							onClick={() => void load(true)}
						>
							Download receipt JSON
						</button>
					</div>
				) : (
					receiptReady && <p>Your household has no departure receipt yet.</p>
				)}
			</div>
			{sessionEnded && <a href="/portal">Open signed-in portal</a>}
		</main>
	);
}
