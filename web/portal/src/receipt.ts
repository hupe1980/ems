export interface Receipt {
	site: string;
	simulation: true;
	delivered_wh: number | null;
	shortfall_wh: number | null;
	evidence: {
		departure: string;
		requested_wh: number;
		opening_wh: number;
		rejoin_wh: number;
		closing_wh: number | null;
	};
}

export async function readReceipt(
	signal: AbortSignal,
): Promise<Receipt | null> {
	const response = await fetch("/v1/me/departure", {
		signal,
		cache: "no-store",
		redirect: "error",
	});
	if (response.status === 204) return null;
	if (response.status === 401 || response.status === 403)
		throw new Error("Your session has ended or access has been withdrawn.");
	if (!response.ok) throw new Error("Your receipt is unavailable. Try again.");
	const data = await response.json();
	const number = (v: unknown) =>
		typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
	const optional = (v: unknown) => v === null || number(v);
	const e = data?.evidence;
	if (
		!e ||
		data.simulation !== true ||
		typeof data.site !== "string" ||
		typeof e.departure !== "string" ||
		!Number.isFinite(Date.parse(e.departure)) ||
		![e.opening_wh, e.rejoin_wh, e.requested_wh].every(number) ||
		![e.closing_wh, data.delivered_wh, data.shortfall_wh].every(optional) ||
		(e.closing_wh === null) !== (data.delivered_wh === null) ||
		(e.closing_wh === null) !== (data.shortfall_wh === null)
	)
		throw new Error("The collector returned an unreadable receipt.");
	return data;
}

export function downloadReceipt(receipt: Receipt) {
	const url = URL.createObjectURL(
		new Blob([JSON.stringify(receipt, null, 2)], { type: "application/json" }),
	);
	const link = document.createElement("a");
	link.href = url;
	link.download = "departure-receipt.json";
	link.click();
	setTimeout(() => URL.revokeObjectURL(url), 1000);
}
