//! Rehearse RTU commissioning on a Unix PTY. The first output line names the tty.
//! Unit 1 accepts `silent`, `online` and `swap` on stdin. Unit 2 keeps reporting.

#[cfg(unix)]
use hemsd::house::registers::Unit;
#[cfg(unix)]
#[path = "serial_meter/peer.rs"]
mod peer;

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use peer::{Meters, Peer};
    use std::io::{BufRead, Write};
    use std::sync::{Arc, Mutex};
    use tokio_modbus::server::rtu::Server;
    use tokio_serial::{SerialPort, SerialStream};

    let (master, slave) = SerialStream::pair()?;
    let path = slave.name().ok_or("missing PTY path")?;
    let meters = Arc::new(Mutex::new(Meters::new()));
    println!("{path}");
    std::io::stdout().flush()?;
    let commands = Arc::clone(&meters);
    let input = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut serial = "grid-001";
        for line in std::io::stdin().lock().lines() {
            let line = line?;
            let command = line.trim();
            let mut plants = commands.lock().expect("meter image lock");
            match command {
                "silent" => plants.grid = None,
                "online" => plants.grid = Some(Meters::grid(serial)),
                "swap" => {
                    serial = if serial == "grid-001" {
                        "grid-002"
                    } else {
                        "grid-001"
                    };
                    plants.grid = Some(Meters::grid(serial));
                }
                _ => {
                    eprintln!("Use silent, online or swap.");
                    continue;
                }
            }
            drop(plants);
            println!("{command}");
            std::io::stdout().flush()?;
        }
        Ok(())
    });
    tokio::select! {
        result = Server::new(master).serve_forever(Peer(meters)) => result?,
        result = input => result??,
    }
    Ok(())
}

#[cfg(not(unix))]
fn main() {
    eprintln!("The serial meter simulator requires a Unix pseudo-terminal.");
}
