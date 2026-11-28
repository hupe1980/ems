import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";
import { departures } from "./fixtures/departures.ts";

export default defineConfig(({ mode }) =>
	mode === "fixtures"
		? {
				plugins: [
					tailwindcss(),
					{
						name: "local-household",
						apply: "serve",
						configureServer(server) {
							server.middlewares.use("/v1/me/departure", (_req, res) => {
								const outcome = process.env.EMS_PORTAL_FIXTURE ?? "short";
								const receipt = departures.get(outcome);
								res.statusCode =
									receipt === null ? 204 : receipt === undefined ? 400 : 200;
								res.setHeader("Content-Type", "application/json");
								res.end(JSON.stringify(receipt));
							});
							server.middlewares.use("/v1/me/history", (_req, res) => {
								res.setHeader("Content-Type", "application/json");
								res.end(
									JSON.stringify(
										Array.from({ length: 14 }, (_, i) => ({
											day: `2026-01-${String(15 - i).padStart(2, "0")}`,
											imported: 9 + ((i * 7) % 13),
											exported: (i * 5) % 8,
											produced: 4 + ((i * 3) % 11),
										})).filter((reading) => reading.day !== "2026-01-11"),
									),
								);
							});
						},
					},
				],
			}
		: {
				plugins: [tailwindcss()],
				server: { proxy: { "/v1/me": "http://127.0.0.1:7680" } },
			},
);
