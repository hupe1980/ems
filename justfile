# ems — task runner (https://just.systems)
#
# `just` on its own lists every recipe.

set shell := ["bash", "-uc"]

# Keep in sync with rust-toolchain.toml and `rust-version` in Cargo.toml.
msrv := "1.99"

# 📋 List all recipes
default:
    @just --list

# Run the workspace CI checks, including database-backed tests.
ci: portal-build fmt-check lint purity test guards publishable deny doc
    @echo "✅ all checks passed"

# Build the customer portal and the chart assets embedded in fleetd.
portal-build:
    npm ci --prefix web/portal --no-audit
    npm run --prefix web/portal lint
    npm run --prefix web/portal build

# Build both application images and exercise all four replacement ports.
interfaces:
    python3 tools/conformance/images.py

# Verify the portal with a disposable PostgreSQL and simulated OCPP outlet.
portal-trial: portal-build
    cargo build --locked -p portald -p obsd -p hemsd --features hemsd/house
    uv run xtask/customer_portal.py target/debug/hemsd target/debug/obsd target/debug/portald

# Measure the winter EV departure with both thermal appliances on its feeder.
house-session-trial:
    cargo test --locked -p hemsd --features house --lib a_winter_departure_is_visible_on_the_occupied_house_meter -- --nocapture

# Commission an OPC UA register on an independent encrypted loopback peer.
opcua-meter-trial:
    cargo build --locked -p cemsd --features opcua
    uv run --script xtask/opcua_meter.py

# Reconcile winter and both Berlin clock-change days over Modbus TCP.
house-meter-trial:
    cargo test --locked -p hemsd --features house --lib berlin_meter_days_reconcile_on_the_wire -- --nocapture

# Drive the virtual battery over model 704 and observe the connection meter.
house-storage-trial:
    cargo test --locked -p hemsd --features house --lib signed_battery_power_reverts_on_the_wire
    cargo test --locked -p hemsd --features house --lib gateway_storage_trial

# Commission a model 203 meter over a pseudo-terminal, then interrupt and restore it.
serial-meter-trial:
    cargo build --locked -p hemsd --features house --bin hemsd --example serial_meter
    uv run --script xtask/serial_meter.py target/debug/hemsd target/debug/examples/serial_meter

# Read a captured wired M-Bus heat meter over a pseudo-terminal and verify Setup.
heat-meter-trial:
    cargo build --locked -p hemsd --bin hemsd
    uv run --script xtask/heat_meter.py target/debug/hemsd

# Compare room and hot-water service under a real S2 controller.
thermal-trial:
    cargo build --locked -p hemsd --features house
    uv run --script xtask/thermal_trial.py target/debug/hemsd

# Rehearse qualified and held gateway updates through the scoped operator desk.
canary-trial:
    cargo build --locked -p histd --bin histd
    cargo build --locked -p fleetd --bin fleetd --example canary_manifest
    uv run --script xtask/canary_browser.py

# Exercise read load while the loopback gateway controls the virtual house.
history-trial:
    cargo build --locked -p hemsd --features house
    python3 xtask/history_load.py target/debug/hemsd

# Exercise the gateway's read-only appliance path through peer loss and restart.
matter-recovery:
    cargo build --locked -p hemsd --features house,matter
    uv run xtask/matter_recovery.py target/debug/hemsd

# Measure 20 exact goal quotes in the browser against the optimized virtual gateway.
quote-trial:
    cargo test --locked --release -p hemsd --test planned_house browser_prices_twenty_departure_targets -- --ignored --nocapture

# Export a populated 1,095-day history while the virtual gateway controls the house.
retained-history-trial:
    cargo build --locked -p hemsd --features house --bin hemsd --example seed_history
    uv run xtask/retained_history.py target/debug/hemsd target/debug/examples/seed_history

# 🎨 Format the workspace
fmt:
    cargo fmt --all

# 🎨 Fail if anything is unformatted
fmt-check:
    cargo fmt --all --check

# 🔍 Clippy, warnings as errors
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Check that the domain crates use no ambient clock, I/O or unsafe code.
purity:
    #!/usr/bin/env bash
    set -uo pipefail
    pure="ems-core ems-device ems-drv ems-flex ems-grid ems-tariff ems-forecast ems-optimizer ems-realtime ems-events"
    fail=0
    for crate in $pure; do
        hits="$(grep -rn --include='*.rs' -E \
            'SystemTime::now|Instant::now|OffsetDateTime::now|std::(fs|env|net|process)|\bunsafe\b' \
            "crates/$crate/src" 2>/dev/null | grep -vE ':[[:space:]]*(///|//!|//)' || true)"
        if [ -n "$hits" ]; then
            echo "❌ $crate reached for ambient state:" >&2
            echo "$hits" >&2
            fail=1
        fi
    done
    [ "$fail" -eq 0 ] && echo "🧊 pure: no clock, no I/O, no unsafe in the domain crates"
    exit "$fail"

# Run all workspace tests. Supply EMS_TEST_POSTGRES or a local container runtime.
test:
    cargo test --workspace --all-features

# Start disposable development PostgreSQL on port 5432.
db:
    docker run --rm -d --name hems-postgres -p 5432:5432 \
        -e POSTGRES_PASSWORD=postgres -e POSTGRES_USER=postgres postgres:18-alpine
    @echo "  export EMS_TEST_POSTGRES=postgres://postgres:postgres@127.0.0.1:5432"

# Remove the development and test database containers.
db-stop:
    -docker rm -f hems-postgres
    -docker rm -f hems-test-postgres

# 🧪 One crate's tests
test-crate crate:
    cargo test -p {{ crate }} --all-features

# Check workspace contracts, manifests, citations, examples and dependency usage.
guards:
    python3 -m unittest discover -s tools/conformance
    python3 tools/conformance/contracts.py
    cargo xtask check-all

# Resolve and package the publishable crates against the registry.
publishable:
    cargo publish --workspace --locked --dry-run --no-verify --allow-dirty

# 📜 Licences and advisories
deny:
    cargo deny check

# 📚 Documentation, warnings as errors
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features

# Validate the household configuration before connecting to devices.
check-box config="services/hemsd/hemsd.example.toml":
    cargo run -p hemsd -- run --check --config {{ config }}

# Run the gateway with the configured devices.
box config="services/hemsd/hemsd.example.toml":
    cargo run -p hemsd -- run --config {{ config }}

# 🏠 One simulated day through the whole control stack
demo day="winter":
    cargo run -p hemsd -- simulate --day {{ day }}

# Print the winter report used in the simulation guide.
day-report:
    cargo run -q -p hemsd --example print_winter_day

demo-all:
    @just demo winter
    @just demo summer
    @just demo deadline
    @just demo shared
    @just demo offline
    @just demo autumn
    @just demo capped
    @echo "  ── the same winter day with the weather known in advance ──"
    @echo "  ── (the same day; only what the planner was told changes) ──"
    cargo run -q -p hemsd -- simulate --day winter --perfect-foresight
    @echo "  ── the same winter day with battery wear priced at zero ──"
    cargo run -q -p hemsd -- simulate --day winter --wear-eur-per-kwh 0
    @echo "  ── the same September day with a fixed three-phase wallbox ──"
    cargo run -q -p hemsd -- simulate --day autumn --no-phase-switching
    @echo "  ── the same capped day with an intelligent meter, which lifts § 9 EEG ──"
    cargo run -q -p hemsd -- simulate --day capped --imsys
    @echo "  ── the shared reduction with every asset weighted the same ──"
    cargo run -q -p hemsd -- simulate --day shared --uniform-weights
    @echo "  ── the same winter day inside a § 42c sharing community ──"
    cargo run -q -p hemsd -- simulate --day winter --sharing

# Simulate the minimum run time of a single-speed heat pump.
demo-on-off day="winter":
    cargo run --release -p hemsd -- simulate --day {{ day }} --heat-pump-on-off

# Compare forecast risk policies across simulated weather conditions.
risk day="deadline" days="4":
    cargo run --release -p hemsd -- risk --day {{ day }} --days {{ days }}

# Measure forecast-band coverage across independent simulated days.
backtest day="summer" days="20":
    cargo run --release -p hemsd -- backtest --day {{ day }} --days {{ days }}

# Dry-run crate publication and smoke-test the three edge binaries locally.
release-check:
    cargo publish --workspace --locked --dry-run
    cargo build --locked --release -p hemsd -p bemsd -p cemsd
    ./target/release/bemsd --version
    ./target/release/cemsd reconcile crates/ems-sim/fixtures/shift-trace.json
    ./target/release/hemsd simulate --day winter
    @echo "Local packaging and edge smoke tests passed."

# 🔒 Minimum supported Rust version
msrv:
    cargo +{{ msrv }} check --workspace --all-features

# Print the advisory queue and reproduce a finding from its inputs.
agent-demo:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo test -q -p agentd --test review -- --nocapture 2>&1 \
        | sed -n '/The advisory queue/,/re-derived/p'
    echo
    echo "  Nothing an agent says moves a watt: Advice is a leaf type nothing"
    echo "  consumes, the authority is derived by attenuation and cannot widen,"
    echo "  and the plane has no route that writes."

# 🛰️ One box reporting a day into the fleet view
[positional-arguments]
fleet-demo day="winter":
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --locked -q -p obsd -p hemsd
    conf="$(mktemp -d)"
    pg=""
    collector=""
    cleanup() {
        if [[ -n "$collector" ]]; then kill "$collector" 2>/dev/null || true; wait "$collector" 2>/dev/null || true; fi
        if [[ -n "$pg" ]]; then docker rm -fv "$pg" >/dev/null; fi
        rm -rf "$conf"
    }
    trap cleanup EXIT
    password="$(openssl rand -hex 24)"
    pg="$(docker run --rm -d -p 127.0.0.1::5432 -e POSTGRES_PASSWORD="$password" postgres:18-alpine)"
    for ((i=0; i<100; i++)); do
        if docker exec "$pg" pg_isready -h 127.0.0.1 >/dev/null; then break; fi
        sleep 0.2
    done
    docker exec "$pg" pg_isready -h 127.0.0.1 >/dev/null
    export DATABASE_URL="postgres://postgres:$password@$(docker port "$pg" 5432/tcp)/postgres"
    export EMS_OBSD_SECRET="whsec_$(openssl rand -base64 32)"
    export EMS_OBSD_OPERATOR_TOKEN="$(openssl rand -hex 24)"
    export EMS_OBSD_LISTEN="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print("127.0.0.1:", s.getsockname()[1], sep="")')"
    origin="http://$EMS_OBSD_LISTEN"
    cat > "$conf/obsd.toml" <<'TOML'
    # The simulator replays fixed calendar dates in this disposable database.
    keep_days = 36525
    [database]
    tls = false
    [webhook_secrets]
    reference-household = ["env:EMS_OBSD_SECRET"]
    [[operators]]
    token = "env:EMS_OBSD_OPERATOR_TOKEN"
    tenant = "*"
    TOML
    ./target/debug/obsd --config "$conf/obsd.toml" &
    collector=$!
    for ((i=0; i<100; i++)); do
        kill -0 "$collector"
        if curl -fs "$origin/readyz" >/dev/null; then break; fi
        sleep 0.2
    done
    curl -fsS "$origin/readyz" >/dev/null
    ./target/debug/hemsd simulate --day "$1" --report-to "$origin"
    curl -fsS -H "Authorization: Bearer $EMS_OBSD_OPERATOR_TOKEN" "$origin/v1/fleet" |
        python3 -c 'import json,sys; view=json.load(sys.stdin); print(json.dumps(view,indent=2)); sys.exit(view["sites"] != 1)'

# Verify the historian's MeterStore register path against PostgreSQL.
histd-registers:
    cargo test --locked -p histd --test meterstore_registers

# Build the browser module and compare its results with the native calculation.
site-wasm:
    #!/usr/bin/env bash
    set -euo pipefail
    rustup target add wasm32-unknown-unknown
    cargo build --locked --release --target wasm32-unknown-unknown -p ems-wasm
    target_dir=$(cargo metadata --locked --no-deps --format-version=1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
    module="$target_dir/wasm32-unknown-unknown/release/ems_wasm.wasm"
    node tools/ems-wasm/parity.mjs "$module"
    cp "$module" site/static/ems.wasm

# Build and check the static site, including the household and industrial simulations.
site-build: site-wasm
    zola --root site check --skip-external-links
    zola --root site build
    python3 xtask/check_site.py

# Serve the documentation and browser simulations locally.
site: site-wasm
    zola --root site serve

# Re-render the social preview from its SVG source.
site-card:
    rsvg-convert -w 1200 -h 630 site/og-card.svg -o site/static/og.png
    @echo "✅ site/static/og.png"
