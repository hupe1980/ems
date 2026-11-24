// Values captured by the two-household OCPP browser trial.
import type { Receipt } from "../src/receipt";

const evidence = {
	departure: "2026-01-15T07:00:00Z",
	opening_wh: 48210,
	requested_wh: 6000,
	rejoin_wh: 50210,
	closing_wh: 52960,
};
export const departures: ReadonlyMap<string, Receipt | null> = new Map<
	string,
	Receipt | null
>([
	[
		"short",
		{
			site: "home",
			simulation: true,
			evidence,
			delivered_wh: 4750,
			shortfall_wh: 1250,
		},
	],
	[
		"partial",
		{
			site: "home",
			simulation: true,
			evidence: { ...evidence, closing_wh: null },
			delivered_wh: null,
			shortfall_wh: null,
		},
	],
	[
		"met",
		{
			site: "second",
			simulation: true,
			evidence: {
				...evidence,
				requested_wh: 3600,
				rejoin_wh: 49410,
				closing_wh: 51810,
			},
			delivered_wh: 3600,
			shortfall_wh: 0,
		},
	],
	["empty", null],
]);
