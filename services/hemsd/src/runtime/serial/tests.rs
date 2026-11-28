use super::*;
use crate::drivers::{Observed, Registry};
use crate::house::registers::Unit;
use ems_core::asset::{Asset, MeterRole};
use ems_core::prelude::{AssetId, Power};
use ems_drv::modbus::{Cadence, Meter as MeterDriver};
use std::sync::{Arc, Mutex};
use tokio_modbus::server::rtu::Server;
use tokio_serial::SerialPort;

#[path = "../../../examples/serial_meter/peer.rs"]
mod simulator;
use simulator::{Meters, Peer};

async fn wait_for(registry: &Shared, grid: &AssetId, test: impl Fn(&Observed) -> bool) -> Observed {
    timeout(Duration::from_secs(8), async {
        loop {
            let observed = registry
                .lock()
                .await
                .observe(Some(grid), OffsetDateTime::now_utc());
            if test(&observed) {
                return observed;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("expected meter state on the RTU bus")
}

fn register_pair() -> (Shared, Vec<Meter>, AssetId, AssetId) {
    let mut house = crate::site::Household::build(&crate::HouseholdConfig::default()).unwrap();
    let grid = house.grid_meter;
    let feeder = AssetId::new("feeder-meter").unwrap();
    let Asset::Meter(mut submeter) = house.site.asset(&grid).unwrap().clone() else {
        panic!("grid connection meter")
    };
    submeter.meta.id = feeder.clone();
    submeter.role = MeterRole::Submeter;
    house.site.assets.push(Asset::Meter(submeter));
    let mut registry = Registry::new();
    let mut devices = Vec::new();
    let cadence = Cadence {
        poll: time::Duration::milliseconds(50),
        timeout: time::Duration::milliseconds(250),
    };
    for (asset, unit) in [(grid.clone(), 1), (feeder.clone(), 2)] {
        let driver = registry
            .register(
                Box::new(MeterDriver::new(asset.clone(), unit, cadence)),
                &house.site,
            )
            .unwrap();
        devices.push(Meter {
            on: Attached { driver, asset },
            unit,
            timeout: Duration::from_millis(250),
        });
    }
    let registry = Arc::new(tokio::sync::Mutex::new(registry));
    (registry, devices, grid, feeder)
}

#[tokio::test]
async fn independent_rtu_meters_isolate_silence_and_recover() {
    let (master, slave) = SerialStream::pair().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meter-bus");
    std::os::unix::fs::symlink(slave.name().unwrap(), &path).unwrap();
    let bus = SerialBus {
        path: path.clone(),
        baud: 19_200,
        parity: SerialParity::None,
        stop_bits: 2,
    };
    let plants = Arc::new(Mutex::new(Meters::new()));
    let peer = tokio::spawn(Server::new(master).serve_forever(Peer(Arc::clone(&plants))));
    let identity = inspect(&bus, 1).await.expect("commissioned over RTU");
    assert_eq!(identity.model, "virtual meter");
    assert_eq!(identity.serial, "grid-001");
    assert!(identity.connection_meter);

    let (registry, devices, grid, feeder) = register_pair();
    let (shutdown, trigger) = Shutdown::channel();
    let owner = tokio::spawn(run(bus, devices, Arc::clone(&registry), shutdown));
    let observed = wait_for(&registry, &grid, |o| {
        o.state.grid.is_some() && o.state.assets.contains_key(&feeder)
    })
    .await;
    let seen = observed.state.grid.unwrap();
    assert_eq!(seen.power.unwrap().get(), -375.);
    assert_eq!(seen.energy_in.unwrap().get(), 45_000.);
    assert_eq!(seen.energy_out.unwrap().get(), 12_000.);
    let first_at = seen.at;

    {
        let mut devices = plants.lock().unwrap();
        devices.grid = None;
        devices.feeder.set_meter(1_100., 124_000., 0.);
    }
    let lost = wait_for(&registry, &grid, |o| {
        o.silent.contains(&grid)
            && o.state.assets.get(&feeder).and_then(|m| m.power) == Some(Power::new(1_100.))
    })
    .await;
    assert!(lost.state.grid.is_none());
    assert!(!lost.silent.contains(&feeder));

    let mut replacement = Unit::meter();
    replacement.set_meter(420., 45_100., 12_050.);
    plants.lock().unwrap().grid = Some(replacement);
    let recovered = wait_for(&registry, &grid, |o| {
        o.state.grid.and_then(|m| m.power) == Some(Power::new(420.))
    })
    .await;
    assert!(recovered.state.grid.unwrap().at > first_at);
    assert_eq!(
        recovered.state.grid.unwrap().energy_in.unwrap().get(),
        45_100.
    );
    std::fs::remove_file(&path).unwrap();
    peer.abort();
    let _ = peer.await;
    drop(slave);
    let disconnected = wait_for(&registry, &grid, |o| {
        o.silent.contains(&grid) && o.silent.contains(&feeder)
    })
    .await;
    assert!(disconnected.state.grid.is_none());
    assert!(!disconnected.state.assets.contains_key(&feeder));

    let (master, slave) = SerialStream::pair().unwrap();
    std::os::unix::fs::symlink(slave.name().unwrap(), &path).unwrap();
    plants
        .lock()
        .unwrap()
        .grid
        .as_mut()
        .unwrap()
        .set_meter(750., 45_110., 12_050.);
    let peer = tokio::spawn(Server::new(master).serve_forever(Peer(plants)));
    wait_for(&registry, &grid, |o| {
        o.state.grid.and_then(|m| m.power) == Some(Power::new(750.)) && !o.silent.contains(&feeder)
    })
    .await;
    trigger.trigger();
    timeout(Duration::from_secs(2), owner)
        .await
        .unwrap()
        .unwrap();
    peer.abort();
    drop(slave);
}

#[tokio::test]
async fn late_and_damaged_frames_cannot_supply_the_next_meter() {
    fn read(unit: u8) -> ReadRequest {
        let tcp = frame::Request {
            unit,
            transaction: u16::from(unit),
            body: frame::RequestBody::Read {
                space: frame::Space::Holding,
                address: 40_000,
                count: 1,
            },
        };
        ReadRequest::from_tcp(&tcp.encode()).unwrap()
    }

    // Independent wire vectors: unit 1 reports 111; unit 2 reports 222.
    let previous = [1, 3, 2, 0, 111, 248, 104];
    let current = [2, 3, 2, 0, 222, 124, 28];
    let (mut peer, mut bus) = SerialStream::pair().unwrap();
    let timing = Timing {
        character_gap: Duration::from_millis(2),
        frame_gap: Duration::from_millis(5),
    };
    let device = tokio::spawn(async move {
        let mut query = [0; 8];
        peer.read_exact(&mut query).await.unwrap();
        assert_eq!(query, read(1).encode());
        peer.read_exact(&mut query).await.unwrap();
        assert_eq!(query, read(2).encode());
        peer.write_all(&previous).await.unwrap();
        sleep(Duration::from_millis(25)).await;
        let mut damaged = current;
        damaged[4] = 220;
        peer.write_all(&damaged).await.unwrap();
        sleep(Duration::from_millis(25)).await;
        peer.write_all(&current[..3]).await.unwrap();
        sleep(Duration::from_millis(25)).await;
        peer.write_all(&current[3..]).await.unwrap();
        sleep(Duration::from_millis(25)).await;
        peer.write_all(&current).await.unwrap();
        peer
    });

    assert!(
        timeout(
            Duration::from_millis(50),
            exchange(&mut bus, &read(1), timing)
        )
        .await
        .is_err()
    );
    let response = timeout(Duration::from_secs(2), exchange(&mut bus, &read(2), timing))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response, read(2).decode_response(&current).unwrap());
    drop(device.await.unwrap());
}
