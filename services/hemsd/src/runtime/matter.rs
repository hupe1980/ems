//! Poll one commissioned appliance through a loopback Matter Server (WebSocket API schema 11).
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_tungstenite::{
    WebSocketStream, connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

use crate::home::AppliancePower;

const CLIENT_SCHEMA: u64 = 11;
const EVERY: Duration = Duration::from_secs(15);
const DEADLINE: Duration = Duration::from_secs(8);

/// Persistent point selection. Pairing and fabric keys belong to Matter Server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    /// Human-readable appliance name.
    pub name: String,
    /// Compressed fabric ID as hexadecimal (also fits unsigned 64-bit IDs in TOML).
    pub fabric: String,
    /// Node ID on that fabric, hexadecimal.
    pub node: String,
    /// Endpoint exposing Electrical Power Measurement.
    pub endpoint: u16,
    /// Matter Server's local WebSocket port.
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_port() -> u16 {
    5580
}

#[derive(Clone, Default)]
struct Sample {
    watts: Option<f64>,
    at: Option<String>,
    seen: Option<Instant>,
    failed: bool,
}

/// Shared observation and immutable selection.
#[derive(Clone)]
pub struct Meter {
    point: Point,
    fabric: u64,
    node: u64,
    path: String,
    sample: watch::Sender<Sample>,
}

impl Meter {
    /// Validate without connecting.
    ///
    /// # Errors
    /// Rejects invalid point identifiers, port or appliance name.
    pub fn new(point: &Point) -> Result<Self> {
        fn id(text: &str) -> Result<u64> {
            let value = u64::from_str_radix(text.strip_prefix("0x").unwrap_or(text), 16)
                .context("Matter IDs must be hexadecimal")?;
            ensure!(value != 0, "Matter IDs must be nonzero");
            Ok(value)
        }
        ensure!(
            !point.name.trim().is_empty() && point.name.len() <= 128,
            "configure a Matter appliance name of 1–128 bytes"
        );
        ensure!(
            point.port != 0 && point.endpoint != u16::MAX,
            "configure a valid Matter port and endpoint"
        );
        Ok(Self {
            point: point.clone(),
            fabric: id(&point.fabric)?,
            node: id(&point.node)?,
            path: ems_drv::matter::active_power_path(point.endpoint),
            sample: watch::channel(Sample::default()).0,
        })
    }

    /// Capture receipt time separately from sensor power. On disconnect retain last evidence.
    pub fn view(&self) -> AppliancePower {
        let sample = self.sample.borrow();
        let state = match sample.seen {
            None if sample.failed => "unavailable",
            None => "waiting",
            Some(t) if sample.failed || t.elapsed() > EVERY + DEADLINE => "stale",
            Some(_) if sample.watts.is_none() => "unknown",
            Some(_) => "current",
        };
        AppliancePower {
            name: self.point.name.clone(),
            node: format!("{:x}", self.node),
            endpoint: self.point.endpoint,
            state,
            watts: sample.watts,
            received_at: sample.at.clone(),
        }
    }

    async fn read(&self) -> Result<Sample> {
        let config = WebSocketConfig::default()
            .max_message_size(Some(16384))
            .max_frame_size(Some(16384));
        let (mut socket, _) = connect_async_with_config(
            format!("ws://127.0.0.1:{}/ws", self.point.port),
            Some(config),
            false,
        )
        .await
        .context("connect to the local Matter Server WebSocket")?;
        let hello = receive(&mut socket).await?;
        ensure!(
            hello["compressed_fabric_id"].as_u64() == Some(self.fabric),
            "compressed fabric ID differs from configuration"
        );
        ensure!(
            hello["schema_version"]
                .as_u64()
                .is_some_and(|v| v >= CLIENT_SCHEMA)
                && hello["min_supported_schema_version"]
                    .as_u64()
                    .is_some_and(|v| v <= CLIENT_SCHEMA),
            "Matter Server does not support API schema {CLIENT_SCHEMA}"
        );
        let request = json!({"message_id":"power","command":"read_attribute","args":{"node_id":self.node,"attribute_path":self.path}});
        socket
            .send(Message::Text(request.to_string().into()))
            .await?;
        let reply = receive(&mut socket).await?;
        ensure!(reply["message_id"] == "power", "unexpected Matter response");
        ensure!(
            reply.get("error_code").is_none(),
            "Matter Server rejected the read; check the node and endpoint"
        );
        let value = reply["result"]
            .get(&self.path)
            .context("Matter read failed or omitted point")?;
        let mw = serde_json::from_value::<Option<i64>>(value.clone())?;
        Ok(Sample {
            watts: ems_drv::matter::watts(mw),
            at: Some(
                time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)?,
            ),
            seen: Some(Instant::now()),
            failed: false,
        })
    }

    /// Read once using the same deadline and selection as continuous polling.
    ///
    /// # Errors
    /// Returns a connection, selection or read error and retains the previous sample.
    pub async fn refresh(&self) -> Result<()> {
        let result = tokio::time::timeout(DEADLINE, self.read())
            .await
            .context("Matter read timed out after 8 seconds")
            .and_then(|read| read);
        match result {
            Ok(sample) => {
                self.sample.send_replace(sample);
                Ok(())
            }
            Err(error) => {
                self.sample.send_modify(|sample| sample.failed = true);
                Err(error)
            }
        }
    }

    /// Read periodically. The daemon cancels this task on shutdown.
    pub async fn poll(&self) {
        let mut tick = tokio::time::interval(EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let _ = self.refresh().await;
        }
    }
}

async fn receive<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
) -> Result<Value> {
    loop {
        match socket
            .next()
            .await
            .context("Matter Server disconnected")??
        {
            Message::Text(text) => return Ok(serde_json::from_str(&text)?),
            Message::Ping(_) => socket.flush().await?,
            Message::Pong(_) => {}
            _ => anyhow::bail!("expected a Matter Server JSON message"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_the_pinned_fabric_and_selected_endpoint_supply_power() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let point = Point {
            name: "Washer".into(),
            fabric: "beef".into(),
            node: "a".into(),
            endpoint: 2,
            port: listener.local_addr().unwrap().port(),
        };
        let meter = Meter::new(&point).unwrap();
        let server = tokio::spawn(async move {
            for (fabric, schema, minimum, id, result) in [
                (48879, 15, 11, "power", json!({"2/144/8":-12500})),
                (1, 15, 11, "power", Value::Null),
                (48879, 10, 9, "power", Value::Null),
                (48879, 16, 12, "power", Value::Null),
                (48879, 15, 11, "power", json!({"1/144/8":0})),
                (48879, 15, 11, "unrelated", json!({"2/144/8":50})),
                (48879, 15, 11, "power", json!({"2/144/8":0})),
                (48879, 15, 11, "power", json!({"2/144/8":null})),
                (48879, 15, 11, "power", json!({"2/144/8":2750})),
            ] {
                let (stream, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let info = json!({"compressed_fabric_id":fabric,"schema_version":schema,"min_supported_schema_version":minimum});
                ws.send(Message::Text(info.to_string().into()))
                    .await
                    .unwrap();
                if fabric != 48879 || !(minimum..=schema).contains(&11) {
                    assert!(receive(&mut ws).await.is_err());
                    continue;
                }
                let req = receive(&mut ws).await.unwrap();
                assert_eq!(
                    req,
                    json!({"message_id":"power","command":"read_attribute","args":{"node_id":10,"attribute_path":"2/144/8"}})
                );
                ws.send(Message::Text(
                    json!({"message_id":id,"result":result}).to_string().into(),
                ))
                .await
                .unwrap();
            }
        });
        assert_eq!(meter.view().state, "waiting");
        for (state, watts) in [
            ("current", Some(-12.5)),
            ("stale", Some(-12.5)),
            ("stale", Some(-12.5)),
            ("stale", Some(-12.5)),
            ("stale", Some(-12.5)),
            ("stale", Some(-12.5)),
            ("current", Some(0.0)),
            ("unknown", None),
        ] {
            assert_eq!(meter.refresh().await.is_ok(), state != "stale");
            assert_eq!((meter.view().state, meter.view().watts), (state, watts));
            assert!(meter.view().received_at.is_some());
        }
        let restarted = Meter::new(&point).unwrap();
        restarted.refresh().await.unwrap();
        assert_eq!(restarted.view().watts, Some(2.75));
        server.await.unwrap();
        assert!(restarted.refresh().await.is_err());
        assert_eq!(restarted.view().state, "stale");
    }
}
