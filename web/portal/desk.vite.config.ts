import { readFileSync } from "node:fs";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

export default defineConfig({
	plugins: [
		tailwindcss(),
		{
			name: "embedded-desk-licenses",
			enforce: "post",
			generateBundle(_, bundle) {
				const script = bundle["history.js"];
				if (script?.type !== "chunk") return;
				const licenses = ["uplot", "tailwindcss"].map((pkg) =>
					readFileSync(`node_modules/${pkg}/LICENSE`, "utf8").trim(),
				);
				script.code = `/*!\n${licenses.join("\n\n")}\n*/\n${script.code}`;
			},
		},
	],
	build: {
		outDir: "../../services/fleetd/src/api/assets",
		emptyOutDir: false,
		lib: {
			entry: "desk/history.ts",
			name: "MeterHistory",
			formats: ["iife"],
			fileName: () => "history.js",
			cssFileName: "desk",
		},
	},
});
